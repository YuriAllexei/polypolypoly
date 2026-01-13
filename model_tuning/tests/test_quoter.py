"""Tests for the InventoryMMQuoter."""

import pytest

from model_tuning.core.models import Inventory, Market, Oracle
from model_tuning.core.quoter import InventoryMMQuoter, QuoterParams
from model_tuning.core.utils import snap_to_tick


class TestSnapToTick:
    """Tests for snap_to_tick utility."""

    def test_snap_up(self) -> None:
        """Should round up to nearest cent."""
        assert snap_to_tick(0.515) == 0.52

    def test_snap_down(self) -> None:
        """Should round down to nearest cent."""
        assert snap_to_tick(0.494) == 0.49

    def test_exact_tick(self) -> None:
        """Exact tick values should be unchanged."""
        assert snap_to_tick(0.50) == 0.50

    def test_sub_cent_precision(self) -> None:
        """Sub-cent precision should be snapped."""
        assert snap_to_tick(0.4875) == 0.49


class TestQuoterParams:
    """Tests for QuoterParams."""

    def test_default_values(self) -> None:
        """Default parameters should be reasonable."""
        params = QuoterParams()
        assert params.base_spread == 0.02
        assert params.edge_threshold == 0.01
        assert params.base_size == 50.0

    def test_custom_values(self) -> None:
        """Custom parameters should be accepted."""
        params = QuoterParams(
            base_spread=0.03,
            gamma_inv=0.7,
        )
        assert params.base_spread == 0.03
        assert params.gamma_inv == 0.7


class TestInventoryMMQuoter:
    """Tests for InventoryMMQuoter."""

    def test_quote_balanced_neutral(
        self,
        quoter: InventoryMMQuoter,
        balanced_inventory: Inventory,
        neutral_market: Market,
        neutral_oracle: Oracle,
    ) -> None:
        """Balanced inventory + neutral oracle should give symmetric quotes."""
        result = quoter.quote(
            inventory=balanced_inventory,
            market=neutral_market,
            oracle=neutral_oracle,
            minutes_to_resolution=10.0,
        )

        # Should quote both sides
        assert result.bid_up is not None
        assert result.bid_down is not None

        # Oracle adjustment should be 0 (neutral oracle)
        assert result.oracle_adj == pytest.approx(0.0, abs=0.001)

        # Imbalance should be 0
        assert result.inventory_q == pytest.approx(0.0)

        # Spread multipliers should be ~1 (balanced)
        assert result.spread_mult_up == pytest.approx(1.0)
        assert result.spread_mult_down == pytest.approx(1.0)

        # Sizes should be equal (balanced)
        assert result.size_up == result.size_down

    def test_quote_bullish_oracle(
        self,
        quoter: InventoryMMQuoter,
        balanced_inventory: Inventory,
        neutral_market: Market,
        bullish_oracle: Oracle,
    ) -> None:
        """Bullish oracle should tighten UP offset and widen DOWN offset."""
        result = quoter.quote(
            inventory=balanced_inventory,
            market=neutral_market,
            oracle=bullish_oracle,
            minutes_to_resolution=10.0,
        )

        # Oracle adjustment should be positive
        assert result.oracle_adj > 0

        # UP offset should be tighter than DOWN offset
        assert result.raw_up_offset < result.raw_down_offset

    def test_quote_overweight_up(
        self,
        quoter: InventoryMMQuoter,
        overweight_up_inventory: Inventory,
        neutral_market: Market,
        neutral_oracle: Oracle,
    ) -> None:
        """Overweight UP should widen UP spread and reduce UP size."""
        result = quoter.quote(
            inventory=overweight_up_inventory,
            market=neutral_market,
            oracle=neutral_oracle,
            minutes_to_resolution=10.0,
        )

        # Imbalance should be positive
        assert result.inventory_q > 0

        # UP spread mult should be > 1 (wider)
        assert result.spread_mult_up > 1.0
        # DOWN spread mult should be < 1 (tighter)
        assert result.spread_mult_down < 1.0

        # UP size should be smaller than DOWN size
        assert result.raw_size_up < result.raw_size_down

    def test_quote_near_resolution(
        self,
        quoter: InventoryMMQuoter,
        balanced_inventory: Inventory,
        neutral_market: Market,
        neutral_oracle: Oracle,
    ) -> None:
        """Near resolution should have higher p_informed and wider spread."""
        result_far = quoter.quote(
            inventory=balanced_inventory,
            market=neutral_market,
            oracle=neutral_oracle,
            minutes_to_resolution=14.0,
        )

        result_near = quoter.quote(
            inventory=balanced_inventory,
            market=neutral_market,
            oracle=neutral_oracle,
            minutes_to_resolution=1.0,
        )

        # p_informed should be higher near resolution
        assert result_near.p_informed > result_far.p_informed

        # Base spread should be wider near resolution
        assert result_near.base_spread > result_far.base_spread

    def test_edge_check_skip(self) -> None:
        """Should skip quoting when edge is too low."""
        params = QuoterParams(edge_threshold=0.05)  # High threshold
        quoter = InventoryMMQuoter(params)

        # Tight market where edge will be low
        tight_market = Market(
            best_ask_up=0.50,
            best_bid_up=0.49,
            best_ask_down=0.51,
            best_bid_down=0.50,
        )

        result = quoter.quote(
            inventory=Inventory(up_qty=100, up_avg=0.48, down_qty=100, down_avg=0.48),
            market=tight_market,
            oracle=Oracle(current_price=97000, threshold=97000),
            minutes_to_resolution=10.0,
        )

        # At least one side should be skipped due to low edge
        assert result.bid_up is None or result.bid_down is None
        # Should have skip reason
        if result.bid_up is None:
            assert result.skip_reason_up is not None
        if result.bid_down is None:
            assert result.skip_reason_down is not None

    def test_from_dict(self) -> None:
        """Should create quoter from dict."""
        config = {
            "base_spread": 0.03,
            "gamma_inv": 0.7,
        }
        quoter = InventoryMMQuoter.from_dict(config)
        assert quoter.params.base_spread == 0.03
        assert quoter.params.gamma_inv == 0.7


class TestAdverseSelection:
    """Tests for adverse selection (Layer 2)."""

    def test_p_informed_decreases_with_time(self) -> None:
        """p_informed should decrease as time increases."""
        quoter = InventoryMMQuoter()

        p1, _ = quoter.calculate_adverse_selection(1.0)
        p5, _ = quoter.calculate_adverse_selection(5.0)
        p10, _ = quoter.calculate_adverse_selection(10.0)

        assert p1 > p5 > p10

    def test_spread_increases_near_resolution(self) -> None:
        """Spread should be wider near resolution."""
        quoter = InventoryMMQuoter()

        _, spread1 = quoter.calculate_adverse_selection(1.0)
        _, spread10 = quoter.calculate_adverse_selection(10.0)

        assert spread1 > spread10

    def test_p_informed_capped(self) -> None:
        """p_informed should be capped at 80%."""
        params = QuoterParams(p_informed_base=0.9)
        quoter = InventoryMMQuoter(params)

        p, _ = quoter.calculate_adverse_selection(0.1)  # Very close to resolution
        assert p <= 0.8


class TestInventorySkew:
    """Tests for inventory skew (Layer 3)."""

    def test_balanced_gives_equal_multipliers(self) -> None:
        """Balanced inventory should give equal spread multipliers."""
        quoter = InventoryMMQuoter()
        balanced = Inventory(up_qty=100, up_avg=0.5, down_qty=100, down_avg=0.5)

        mult_up, mult_down, size_up, size_down = quoter.calculate_inventory_skew(
            balanced
        )

        assert mult_up == pytest.approx(1.0)
        assert mult_down == pytest.approx(1.0)
        assert size_up == pytest.approx(size_down)

    def test_overweight_up_adjusts_correctly(self) -> None:
        """Overweight UP should increase UP mult and decrease DOWN mult."""
        quoter = InventoryMMQuoter()
        overweight = Inventory(up_qty=150, up_avg=0.5, down_qty=50, down_avg=0.5)

        mult_up, mult_down, size_up, size_down = quoter.calculate_inventory_skew(
            overweight
        )

        assert mult_up > 1.0  # Wider UP spread
        assert mult_down < 1.0  # Tighter DOWN spread
        assert size_up < size_down  # Smaller UP orders
