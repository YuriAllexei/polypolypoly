"""Tests for core data models."""

import pytest

from model_tuning.core.models import Inventory, Market, Oracle, QuoteResult


class TestInventory:
    """Tests for Inventory model."""

    def test_combined_avg(self) -> None:
        """Combined avg should be sum of up_avg and down_avg."""
        inv = Inventory(up_qty=100, up_avg=0.48, down_qty=100, down_avg=0.50)
        assert inv.combined_avg == pytest.approx(0.98)

    def test_imbalance_balanced(self) -> None:
        """Imbalance should be 0 when equal quantities."""
        inv = Inventory(up_qty=100, up_avg=0.50, down_qty=100, down_avg=0.50)
        assert inv.imbalance == pytest.approx(0.0)

    def test_imbalance_overweight_up(self) -> None:
        """Imbalance should be positive when overweight UP."""
        inv = Inventory(up_qty=150, up_avg=0.50, down_qty=50, down_avg=0.50)
        assert inv.imbalance == pytest.approx(0.5)  # (150-50)/(150+50) = 0.5

    def test_imbalance_overweight_down(self) -> None:
        """Imbalance should be negative when overweight DOWN."""
        inv = Inventory(up_qty=50, up_avg=0.50, down_qty=150, down_avg=0.50)
        assert inv.imbalance == pytest.approx(-0.5)

    def test_imbalance_empty(self) -> None:
        """Imbalance should be 0 when empty."""
        inv = Inventory(up_qty=0, up_avg=0.50, down_qty=0, down_avg=0.50)
        assert inv.imbalance == 0.0

    def test_pairs(self) -> None:
        """Pairs should be min of up and down quantities."""
        inv = Inventory(up_qty=100, up_avg=0.50, down_qty=80, down_avg=0.50)
        assert inv.pairs == 80

    def test_potential_profit_positive(self) -> None:
        """Profit should be positive when combined_avg < 1."""
        inv = Inventory(up_qty=100, up_avg=0.48, down_qty=100, down_avg=0.48)
        assert inv.potential_profit == pytest.approx(0.04)

    def test_potential_profit_negative(self) -> None:
        """Profit should be negative when combined_avg > 1 (underwater)."""
        inv = Inventory(up_qty=100, up_avg=0.55, down_qty=100, down_avg=0.50)
        assert inv.potential_profit == pytest.approx(-0.05)

    def test_update_position_up(self) -> None:
        """Updating UP position should correctly update qty and avg."""
        inv = Inventory(up_qty=100, up_avg=0.48, down_qty=100, down_avg=0.50)
        new_inv = inv.update_position("up", 50, 0.52)

        assert new_inv.up_qty == 150
        # New avg: (100*0.48 + 50*0.52) / 150 = (48 + 26) / 150 = 0.4933...
        expected_avg = (100 * 0.48 + 50 * 0.52) / 150
        assert new_inv.up_avg == pytest.approx(expected_avg)
        # Down unchanged
        assert new_inv.down_qty == 100
        assert new_inv.down_avg == 0.50

    def test_update_position_down(self) -> None:
        """Updating DOWN position should correctly update qty and avg."""
        inv = Inventory(up_qty=100, up_avg=0.48, down_qty=100, down_avg=0.50)
        new_inv = inv.update_position("down", 50, 0.45)

        assert new_inv.down_qty == 150
        expected_avg = (100 * 0.50 + 50 * 0.45) / 150
        assert new_inv.down_avg == pytest.approx(expected_avg)
        # Up unchanged
        assert new_inv.up_qty == 100
        assert new_inv.up_avg == 0.48


class TestMarket:
    """Tests for Market model."""

    def test_overround(self) -> None:
        """Overround should be asks sum minus 1."""
        mkt = Market(
            best_ask_up=0.56,
            best_bid_up=0.54,
            best_ask_down=0.46,
            best_bid_down=0.44,
        )
        # Asks sum to 1.02, so overround = 0.02
        assert mkt.overround == pytest.approx(0.02)

    def test_underround(self) -> None:
        """Underround should be 1 minus bids sum."""
        mkt = Market(
            best_ask_up=0.56,
            best_bid_up=0.54,
            best_ask_down=0.46,
            best_bid_down=0.44,
        )
        # Bids sum to 0.98, so underround = 0.02
        assert mkt.underround == pytest.approx(0.02)


class TestOracle:
    """Tests for Oracle model."""

    def test_distance_pct_above(self) -> None:
        """Distance should be positive when above threshold."""
        oracle = Oracle(current_price=97200, threshold=97000)
        assert oracle.distance_pct == pytest.approx(200 / 97000)

    def test_distance_pct_below(self) -> None:
        """Distance should be negative when below threshold."""
        oracle = Oracle(current_price=96800, threshold=97000)
        assert oracle.distance_pct == pytest.approx(-200 / 97000)

    def test_distance_pct_at(self) -> None:
        """Distance should be 0 when at threshold."""
        oracle = Oracle(current_price=97000, threshold=97000)
        assert oracle.distance_pct == pytest.approx(0.0)

    def test_direction_above(self) -> None:
        """Direction should be ABOVE when price > threshold."""
        oracle = Oracle(current_price=97200, threshold=97000)
        assert oracle.direction == "ABOVE"

    def test_direction_below(self) -> None:
        """Direction should be BELOW when price < threshold."""
        oracle = Oracle(current_price=96800, threshold=97000)
        assert oracle.direction == "BELOW"

    def test_direction_at(self) -> None:
        """Direction should be AT when price == threshold."""
        oracle = Oracle(current_price=97000, threshold=97000)
        assert oracle.direction == "AT"


class TestQuoteResult:
    """Tests for QuoteResult model."""

    def test_combined_bid_both_sides(self) -> None:
        """Combined bid when quoting both sides."""
        result = QuoteResult(
            bid_up=0.48,
            size_up=50,
            bid_down=0.48,
            size_down=50,
            oracle_adj=0.01,
            raw_up_offset=0.02,
            raw_down_offset=0.02,
            p_informed=0.1,
            base_spread=0.02,
            inventory_q=0.0,
            spread_mult_up=1.0,
            spread_mult_down=1.0,
            final_up_offset=0.02,
            final_down_offset=0.02,
            raw_size_up=50,
            raw_size_down=50,
            edge_up=0.02,
            edge_down=0.02,
        )
        assert result.combined_bid == pytest.approx(0.96)

    def test_combined_bid_one_side_skipped(self) -> None:
        """Combined bid is None when one side skipped."""
        result = QuoteResult(
            bid_up=0.48,
            size_up=50,
            bid_down=None,
            size_down=0,
            oracle_adj=0.01,
            raw_up_offset=0.02,
            raw_down_offset=0.02,
            p_informed=0.1,
            base_spread=0.02,
            inventory_q=0.0,
            spread_mult_up=1.0,
            spread_mult_down=1.0,
            final_up_offset=0.02,
            final_down_offset=0.02,
            raw_size_up=50,
            raw_size_down=50,
            edge_up=0.02,
            edge_down=0.001,
            skip_reason_down="low edge",
        )
        assert result.combined_bid is None

    def test_profit_per_pair(self) -> None:
        """Profit per pair calculation."""
        result = QuoteResult(
            bid_up=0.48,
            size_up=50,
            bid_down=0.48,
            size_down=50,
            oracle_adj=0.01,
            raw_up_offset=0.02,
            raw_down_offset=0.02,
            p_informed=0.1,
            base_spread=0.02,
            inventory_q=0.0,
            spread_mult_up=1.0,
            spread_mult_down=1.0,
            final_up_offset=0.02,
            final_down_offset=0.02,
            raw_size_up=50,
            raw_size_down=50,
            edge_up=0.02,
            edge_down=0.02,
        )
        # Combined bid = 0.96, profit = 1.0 - 0.96 = 0.04
        assert result.profit_per_pair == pytest.approx(0.04)
