"""Tests for the Backtester."""

import pytest

from model_tuning.core.models import Inventory
from model_tuning.core.quoter import InventoryMMQuoter
from model_tuning.data.loaders import generate_synthetic_ticks
from model_tuning.tuning.backtester import Backtester, FillSimulator, MarketTick


class TestFillSimulator:
    """Tests for FillSimulator."""

    def test_no_fill_with_negative_edge(self) -> None:
        """Should never fill with negative edge."""
        sim = FillSimulator(random_seed=42)

        # Bid above ask = negative edge
        filled, qty = sim.simulate_fill(bid=0.55, market_ask=0.50, size=100)

        assert not filled
        assert qty == 0.0

    def test_fill_probability_increases_with_edge(self) -> None:
        """Higher edge should give more fills on average."""
        sim_low = FillSimulator(random_seed=42)
        sim_high = FillSimulator(random_seed=42)

        # Count fills over many trials
        low_edge_fills = 0
        high_edge_fills = 0

        for _ in range(100):
            if sim_low.simulate_fill(0.48, 0.50, 100)[0]:  # 2c edge
                low_edge_fills += 1
            if sim_high.simulate_fill(0.45, 0.50, 100)[0]:  # 5c edge
                high_edge_fills += 1

        # High edge should get more fills (statistically)
        # With seed=42, this should be deterministic
        assert high_edge_fills >= low_edge_fills

    def test_fill_quantity_is_partial(self) -> None:
        """Fill quantity should be between 50-100% of order size."""
        sim = FillSimulator(random_seed=42, base_fill_prob=1.0)  # Always fill

        filled, qty = sim.simulate_fill(bid=0.45, market_ask=0.50, size=100)

        if filled:
            assert 50 <= qty <= 100


class TestBacktester:
    """Tests for Backtester."""

    def test_backtest_runs(
        self,
        quoter: InventoryMMQuoter,
        synthetic_ticks: list[MarketTick],
        backtester: Backtester,
    ) -> None:
        """Backtest should complete without errors."""
        result = backtester.run(quoter, synthetic_ticks)

        # Should have results
        assert result.metrics is not None
        assert len(result.pnl_history) == len(synthetic_ticks)
        assert len(result.inventory_history) == len(synthetic_ticks)

    def test_backtest_with_initial_inventory(self) -> None:
        """Backtest should respect initial inventory."""
        initial = Inventory(up_qty=50, up_avg=0.48, down_qty=50, down_avg=0.48)
        backtester = Backtester(
            fill_simulator=FillSimulator(random_seed=42),
            initial_inventory=initial,
        )

        ticks = generate_synthetic_ticks(duration_minutes=1.0, random_seed=42)
        quoter = InventoryMMQuoter()

        result = backtester.run(quoter, ticks)

        # Final inventory should have at least the initial amount (plus any fills)
        final_up, final_down = result.inventory_history[-1]
        assert final_up >= 50 or final_down >= 50  # At least one side grew

    def test_backtest_fills_recorded(
        self,
        quoter: InventoryMMQuoter,
        synthetic_ticks: list[MarketTick],
    ) -> None:
        """Fills should be recorded in backtest result."""
        # Use high fill probability for testing
        backtester = Backtester(
            fill_simulator=FillSimulator(
                base_fill_prob=0.8,
                random_seed=42,
            ),
        )

        result = backtester.run(quoter, synthetic_ticks)

        # Should have some fills
        assert len(result.fills) > 0

        # Each fill should have required fields
        for fill in result.fills:
            assert fill.side in ("up", "down")
            assert fill.qty > 0
            assert 0 < fill.price < 1

    def test_backtest_metrics_calculated(
        self,
        quoter: InventoryMMQuoter,
        synthetic_ticks: list[MarketTick],
        backtester: Backtester,
    ) -> None:
        """Metrics should be calculated from backtest."""
        result = backtester.run(quoter, synthetic_ticks)

        metrics = result.metrics

        # Should have all expected metrics
        assert metrics.total_fills >= 0
        assert 0 <= metrics.fill_rate <= 100
        assert metrics.max_drawdown >= 0
        assert -1 <= metrics.final_imbalance <= 1


class TestBacktestResult:
    """Tests for BacktestResult structure."""

    def test_params_stored(
        self,
        synthetic_ticks: list[MarketTick],
        backtester: Backtester,
    ) -> None:
        """Quoter params should be stored in result."""
        from model_tuning.core.quoter import QuoterParams

        params = QuoterParams(base_spread=0.03)
        quoter = InventoryMMQuoter(params)

        result = backtester.run(quoter, synthetic_ticks)

        assert result.params is not None
        assert result.params.base_spread == 0.03


class TestGenerateSyntheticTicks:
    """Tests for synthetic tick generation."""

    def test_generates_correct_count(self) -> None:
        """Should generate correct number of ticks."""
        ticks = generate_synthetic_ticks(
            duration_minutes=5.0,
            tick_interval_seconds=5.0,
        )

        # 5 minutes = 300 seconds, at 5 second intervals = 60 ticks
        assert len(ticks) == 60

    def test_minutes_to_resolution_decreases(self) -> None:
        """Minutes to resolution should decrease over time."""
        ticks = generate_synthetic_ticks(duration_minutes=5.0)

        assert ticks[0].minutes_to_resolution > ticks[-1].minutes_to_resolution
        assert ticks[-1].minutes_to_resolution < 0.1  # Near zero at end

    def test_prices_are_valid(self) -> None:
        """All prices should be valid (0-1 range)."""
        ticks = generate_synthetic_ticks(duration_minutes=5.0)

        for tick in ticks:
            assert 0 < tick.best_ask_up < 1
            assert 0 < tick.best_bid_up < 1
            assert 0 < tick.best_ask_down < 1
            assert 0 < tick.best_bid_down < 1
            assert tick.best_ask_up > tick.best_bid_up
            assert tick.best_ask_down > tick.best_bid_down

    def test_reproducible_with_seed(self) -> None:
        """Same seed should produce same ticks."""
        ticks1 = generate_synthetic_ticks(duration_minutes=1.0, random_seed=42)
        ticks2 = generate_synthetic_ticks(duration_minutes=1.0, random_seed=42)

        assert len(ticks1) == len(ticks2)
        for t1, t2 in zip(ticks1, ticks2, strict=True):
            assert t1.oracle_price == t2.oracle_price
            assert t1.best_ask_up == t2.best_ask_up
