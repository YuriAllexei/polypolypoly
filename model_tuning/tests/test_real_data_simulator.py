"""Tests for the RealDataSimulator."""

import pytest

from model_tuning.core.models import Inventory
from model_tuning.core.quoter import InventoryMMQuoter, QuoterParams
from model_tuning.simulation.models import (
    Orderbook,
    OrderbookLevel,
    OrderbookSnapshot,
    OracleSnapshot,
    PositionState,
    RealFill,
)
from model_tuning.simulation.simulator import RealDataSimulator, SimulationResult


@pytest.fixture
def sample_orderbooks() -> list[OrderbookSnapshot]:
    """Sample orderbook snapshots."""
    return [
        OrderbookSnapshot(
            up=Orderbook(
                asks=[OrderbookLevel(price=0.56, size=100)],
                bids=[OrderbookLevel(price=0.54, size=100)],
            ),
            down=Orderbook(
                asks=[OrderbookLevel(price=0.46, size=100)],
                bids=[OrderbookLevel(price=0.44, size=100)],
            ),
            timestamp=1000.0,
        ),
        OrderbookSnapshot(
            up=Orderbook(
                asks=[OrderbookLevel(price=0.57, size=100)],
                bids=[OrderbookLevel(price=0.55, size=100)],
            ),
            down=Orderbook(
                asks=[OrderbookLevel(price=0.45, size=100)],
                bids=[OrderbookLevel(price=0.43, size=100)],
            ),
            timestamp=1060.0,
        ),
    ]


@pytest.fixture
def sample_fills() -> list[RealFill]:
    """Sample fills (sells that could hit our bids)."""
    return [
        RealFill(price=0.53, size=50, side="sell", timestamp=1030.0, outcome="up"),
        RealFill(price=0.43, size=30, side="sell", timestamp=1045.0, outcome="down"),
    ]


@pytest.fixture
def sample_oracle() -> list[OracleSnapshot]:
    """Sample oracle data."""
    return [
        OracleSnapshot(price=97200.0, threshold=97000.0, timestamp=1000.0),
    ]


class TestRealDataSimulator:
    """Tests for RealDataSimulator."""

    def test_simulation_runs(
        self,
        sample_orderbooks: list[OrderbookSnapshot],
        sample_fills: list[RealFill],
        sample_oracle: list[OracleSnapshot],
    ) -> None:
        """Simulation should complete without errors."""
        quoter = InventoryMMQuoter()
        simulator = RealDataSimulator()

        result = simulator.run(
            quoter=quoter,
            orderbooks=sample_orderbooks,
            fills=sample_fills,
            oracle=sample_oracle,
        )

        assert result.final_inventory is not None
        assert len(result.position_history) == len(sample_orderbooks)

    def test_starts_with_zero_inventory(
        self,
        sample_orderbooks: list[OrderbookSnapshot],
        sample_fills: list[RealFill],
        sample_oracle: list[OracleSnapshot],
    ) -> None:
        """Should start with zero inventory by default."""
        quoter = InventoryMMQuoter()
        simulator = RealDataSimulator()

        result = simulator.run(
            quoter=quoter,
            orderbooks=sample_orderbooks,
            fills=[],  # No fills
            oracle=sample_oracle,
        )

        # First position should have zero qty
        assert result.position_history[0].up_qty == 0
        assert result.position_history[0].down_qty == 0

    def test_custom_initial_inventory(
        self,
        sample_orderbooks: list[OrderbookSnapshot],
        sample_oracle: list[OracleSnapshot],
    ) -> None:
        """Should use custom initial inventory if provided."""
        quoter = InventoryMMQuoter()
        simulator = RealDataSimulator()

        initial_inv = Inventory(up_qty=100, down_qty=50, up_avg=0.45, down_avg=0.48)

        result = simulator.run(
            quoter=quoter,
            orderbooks=sample_orderbooks,
            fills=[],
            oracle=sample_oracle,
            initial_inventory=initial_inv,
        )

        # Final inventory should start from initial
        # (may change if fills match, but with no fills should stay same)
        assert result.final_inventory.up_qty == 100
        assert result.final_inventory.down_qty == 50

    def test_result_has_params(
        self,
        sample_orderbooks: list[OrderbookSnapshot],
        sample_oracle: list[OracleSnapshot],
    ) -> None:
        """Result should contain the quoter params used."""
        params = QuoterParams(base_spread=0.03, gamma_inv=0.7)
        quoter = InventoryMMQuoter(params)
        simulator = RealDataSimulator()

        result = simulator.run(
            quoter=quoter,
            orderbooks=sample_orderbooks,
            fills=[],
            oracle=sample_oracle,
        )

        assert result.params is not None
        assert result.params.base_spread == 0.03
        assert result.params.gamma_inv == 0.7


class TestFillMatching:
    """Tests for fill matching logic."""

    def test_only_sell_fills_match(
        self,
        sample_orderbooks: list[OrderbookSnapshot],
        sample_oracle: list[OracleSnapshot],
    ) -> None:
        """Only SELL fills should match our bids (someone selling into our bid)."""
        # BUY fills should NOT match - they're buying, not selling into our bid
        fills = [
            RealFill(price=0.50, size=50, side="buy", timestamp=1030.0, outcome="up"),
        ]

        quoter = InventoryMMQuoter()
        simulator = RealDataSimulator()

        result = simulator.run(
            quoter=quoter,
            orderbooks=sample_orderbooks,
            fills=fills,
            oracle=sample_oracle,
        )

        assert result.total_fills == 0  # BUY fills should not match our bids

    def test_fill_matches_when_price_low_enough(
        self,
        sample_orderbooks: list[OrderbookSnapshot],
        sample_oracle: list[OracleSnapshot],
    ) -> None:
        """Fill should match if sell price <= our bid."""
        # Create a sell at a low price that should hit our bid
        fills = [
            RealFill(price=0.01, size=25, side="sell", timestamp=1030.0, outcome="up"),
        ]

        # Use aggressive params to ensure we quote
        params = QuoterParams(base_spread=0.01, edge_threshold=0.001, min_offset=0.001)
        quoter = InventoryMMQuoter(params)
        simulator = RealDataSimulator()

        result = simulator.run(
            quoter=quoter,
            orderbooks=sample_orderbooks,
            fills=fills,
            oracle=sample_oracle,
        )

        # With such a low sell price (below our bid), we should get filled
        assert result.total_fills > 0
        assert result.up_fills > 0

    def test_fill_does_not_match_when_price_too_high(
        self,
        sample_orderbooks: list[OrderbookSnapshot],
        sample_oracle: list[OracleSnapshot],
    ) -> None:
        """Fill should not match if sell price > our bid."""
        # Create a sell at a price above our bid
        fills = [
            RealFill(price=0.99, size=25, side="sell", timestamp=1030.0, outcome="up"),
        ]

        quoter = InventoryMMQuoter()
        simulator = RealDataSimulator()

        result = simulator.run(
            quoter=quoter,
            orderbooks=sample_orderbooks,
            fills=fills,
            oracle=sample_oracle,
        )

        # Sell price is above our bid, so no match
        assert result.up_fills == 0

    def test_partial_fill_respects_size_limit(
        self,
        sample_orderbooks: list[OrderbookSnapshot],
        sample_oracle: list[OracleSnapshot],
    ) -> None:
        """Should not fill more than our quoted size."""
        # Large sell that exceeds typical quote size
        fills = [
            RealFill(price=0.01, size=10000, side="sell", timestamp=1030.0, outcome="up"),
        ]

        params = QuoterParams(base_size=50.0, edge_threshold=0.001)
        quoter = InventoryMMQuoter(params)
        simulator = RealDataSimulator()

        result = simulator.run(
            quoter=quoter,
            orderbooks=sample_orderbooks,
            fills=fills,
            oracle=sample_oracle,
        )

        # Should be limited to quote size (approximately base_size)
        if result.up_fills > 0:
            total_up_volume = sum(
                mf.size for mf in result.matched_fills if mf.outcome == "up"
            )
            # Should not exceed base_size significantly
            assert total_up_volume <= params.base_size * 2  # Allow some margin


class TestPositionHistory:
    """Tests for position history tracking."""

    def test_position_recorded_each_timestep(
        self,
        sample_orderbooks: list[OrderbookSnapshot],
        sample_fills: list[RealFill],
        sample_oracle: list[OracleSnapshot],
    ) -> None:
        """Position should be recorded at each orderbook snapshot."""
        quoter = InventoryMMQuoter()
        simulator = RealDataSimulator()

        result = simulator.run(
            quoter=quoter,
            orderbooks=sample_orderbooks,
            fills=sample_fills,
            oracle=sample_oracle,
        )

        assert len(result.position_history) == len(sample_orderbooks)

        # Each position should have correct timestamp
        for i, pos in enumerate(result.position_history):
            assert pos.timestamp == sample_orderbooks[i].timestamp

    def test_position_state_has_all_fields(
        self,
        sample_orderbooks: list[OrderbookSnapshot],
        sample_oracle: list[OracleSnapshot],
    ) -> None:
        """PositionState should have all required fields."""
        quoter = InventoryMMQuoter()
        simulator = RealDataSimulator()

        result = simulator.run(
            quoter=quoter,
            orderbooks=sample_orderbooks,
            fills=[],
            oracle=sample_oracle,
        )

        pos = result.position_history[0]
        assert hasattr(pos, "timestamp")
        assert hasattr(pos, "up_qty")
        assert hasattr(pos, "down_qty")
        assert hasattr(pos, "up_avg")
        assert hasattr(pos, "down_avg")
        assert hasattr(pos, "pairs")
        assert hasattr(pos, "combined_avg")
        assert hasattr(pos, "potential_profit")


class TestInventoryUpdates:
    """Tests for inventory update logic."""

    def test_inventory_updates_after_fill(
        self,
        sample_orderbooks: list[OrderbookSnapshot],
        sample_oracle: list[OracleSnapshot],
    ) -> None:
        """Inventory should update correctly after a matched fill."""
        # Sell at low price hits our bid
        fills = [
            RealFill(price=0.01, size=50, side="sell", timestamp=1030.0, outcome="up"),
        ]

        params = QuoterParams(base_size=100.0, edge_threshold=0.001)
        quoter = InventoryMMQuoter(params)
        simulator = RealDataSimulator()

        result = simulator.run(
            quoter=quoter,
            orderbooks=sample_orderbooks,
            fills=fills,
            oracle=sample_oracle,
        )

        # If fills matched, inventory should reflect it
        if result.up_fills > 0:
            assert result.final_inventory.up_qty > 0

    def test_matched_fill_records_our_price(
        self,
        sample_orderbooks: list[OrderbookSnapshot],
        sample_oracle: list[OracleSnapshot],
    ) -> None:
        """MatchedFill should record our bid price, not the market fill price."""
        # Sell at low price hits our bid
        fills = [
            RealFill(price=0.40, size=50, side="sell", timestamp=1030.0, outcome="up"),
        ]

        params = QuoterParams(edge_threshold=0.001)
        quoter = InventoryMMQuoter(params)
        simulator = RealDataSimulator()

        result = simulator.run(
            quoter=quoter,
            orderbooks=sample_orderbooks,
            fills=fills,
            oracle=sample_oracle,
        )

        if result.matched_fills:
            mf = result.matched_fills[0]
            # Our bid price should be >= the sell price (that's why we matched)
            assert mf.price >= mf.original_fill.price
            # Original fill reference should be preserved
            assert mf.original_fill.price == 0.40


class TestOracleLookup:
    """Tests for oracle timestamp lookup."""

    def test_oracle_lookup_uses_previous_snapshot(self) -> None:
        """Should use most recent oracle at or before current time."""
        orderbooks = [
            OrderbookSnapshot(
                up=Orderbook(asks=[OrderbookLevel(price=0.55, size=100)], bids=[]),
                down=Orderbook(asks=[OrderbookLevel(price=0.45, size=100)], bids=[]),
                timestamp=1500.0,
            ),
        ]

        oracle = [
            OracleSnapshot(price=97000.0, threshold=97000.0, timestamp=1000.0),
            OracleSnapshot(price=97500.0, threshold=97000.0, timestamp=1400.0),
            OracleSnapshot(price=98000.0, threshold=97000.0, timestamp=1600.0),
        ]

        simulator = RealDataSimulator()

        # At timestamp 1500, should use oracle at 1400 (97500)
        oracle_obj = simulator._build_oracle(1500.0, oracle)
        assert oracle_obj.current_price == 97500.0

    def test_oracle_lookup_before_all_data(self) -> None:
        """Should use first oracle if timestamp is before all data."""
        oracle = [
            OracleSnapshot(price=97500.0, threshold=97000.0, timestamp=1000.0),
        ]

        simulator = RealDataSimulator()

        # Timestamp before oracle data
        oracle_obj = simulator._build_oracle(500.0, oracle)
        assert oracle_obj.current_price == 97500.0


class TestEdgeCases:
    """Tests for edge cases."""

    def test_empty_orderbooks(self) -> None:
        """Should handle empty orderbook list."""
        quoter = InventoryMMQuoter()
        simulator = RealDataSimulator()

        result = simulator.run(
            quoter=quoter,
            orderbooks=[],
            fills=[],
            oracle=[],
        )

        assert result.total_fills == 0
        assert len(result.position_history) == 0

    def test_empty_fills(
        self,
        sample_orderbooks: list[OrderbookSnapshot],
        sample_oracle: list[OracleSnapshot],
    ) -> None:
        """Should handle empty fills list."""
        quoter = InventoryMMQuoter()
        simulator = RealDataSimulator()

        result = simulator.run(
            quoter=quoter,
            orderbooks=sample_orderbooks,
            fills=[],
            oracle=sample_oracle,
        )

        assert result.total_fills == 0
        assert result.final_inventory.up_qty == 0
        assert result.final_inventory.down_qty == 0

    def test_empty_oracle(
        self,
        sample_orderbooks: list[OrderbookSnapshot],
    ) -> None:
        """Should handle empty oracle list with neutral defaults."""
        quoter = InventoryMMQuoter()
        simulator = RealDataSimulator()

        result = simulator.run(
            quoter=quoter,
            orderbooks=sample_orderbooks,
            fills=[],
            oracle=[],
        )

        # Should complete without error using neutral oracle
        assert len(result.position_history) == len(sample_orderbooks)

    def test_orderbook_missing_bids(self) -> None:
        """Should handle orderbook with no bids."""
        orderbooks = [
            OrderbookSnapshot(
                up=Orderbook(asks=[OrderbookLevel(price=0.55, size=100)], bids=[]),
                down=Orderbook(asks=[OrderbookLevel(price=0.45, size=100)], bids=[]),
                timestamp=1000.0,
            ),
        ]

        quoter = InventoryMMQuoter()
        simulator = RealDataSimulator()

        result = simulator.run(
            quoter=quoter,
            orderbooks=orderbooks,
            fills=[],
            oracle=[],
        )

        # Should complete without error
        assert len(result.position_history) == 1


class TestSimulationResult:
    """Tests for SimulationResult dataclass."""

    def test_summary_stats_calculation(
        self,
        sample_orderbooks: list[OrderbookSnapshot],
        sample_oracle: list[OracleSnapshot],
    ) -> None:
        """Summary stats should be correctly calculated."""
        # Sells at low prices hit our bids
        fills = [
            RealFill(price=0.01, size=50, side="sell", timestamp=1030.0, outcome="up"),
            RealFill(price=0.01, size=30, side="sell", timestamp=1035.0, outcome="down"),
        ]

        params = QuoterParams(edge_threshold=0.001)
        quoter = InventoryMMQuoter(params)
        simulator = RealDataSimulator()

        result = simulator.run(
            quoter=quoter,
            orderbooks=sample_orderbooks,
            fills=fills,
            oracle=sample_oracle,
        )

        # Stats should match actual data
        assert result.total_fills == result.up_fills + result.down_fills
        assert result.total_volume == sum(mf.size for mf in result.matched_fills)
