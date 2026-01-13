"""Backtester for simulating quoter performance against historical data."""

import random
from dataclasses import dataclass, field

from pydantic import BaseModel

from model_tuning.core.models import Inventory, Market, Oracle
from model_tuning.core.quoter import InventoryMMQuoter, QuoterParams
from model_tuning.tuning.metrics import MetricsSummary, calculate_metrics


class MarketTick(BaseModel):
    """A single market data point for backtesting."""

    timestamp: float
    """Timestamp (can be minutes from start or epoch)."""

    oracle_price: float
    """Oracle price at this tick."""

    threshold: float
    """Market question threshold."""

    best_ask_up: float
    """Best ask for UP."""

    best_bid_up: float
    """Best bid for UP."""

    best_ask_down: float
    """Best ask for DOWN."""

    best_bid_down: float
    """Best bid for DOWN."""

    minutes_to_resolution: float
    """Time remaining until resolution."""


@dataclass
class FillRecord:
    """Record of a fill during backtesting."""

    timestamp: float
    side: str  # "up" or "down"
    qty: float
    price: float
    spread_captured: float


@dataclass
class BacktestResult:
    """Results from a backtest run."""

    metrics: MetricsSummary
    """Summary metrics."""

    fills: list[FillRecord] = field(default_factory=list)
    """All fills during the backtest."""

    pnl_history: list[float] = field(default_factory=list)
    """PnL at each timestep."""

    inventory_history: list[tuple[float, float]] = field(default_factory=list)
    """(up_qty, down_qty) at each timestep."""

    params: QuoterParams | None = None
    """Parameters used for this backtest."""


class FillSimulator:
    """Simulates order fills based on market conditions.

    Simple model: fills occur probabilistically based on edge.
    More edge = higher fill probability.
    """

    def __init__(
        self,
        base_fill_prob: float = 0.3,
        edge_sensitivity: float = 10.0,
        random_seed: int | None = None,
    ) -> None:
        """Initialize fill simulator.

        Args:
            base_fill_prob: Base probability of fill at 1c edge
            edge_sensitivity: How much edge affects fill prob
            random_seed: Random seed for reproducibility
        """
        self.base_fill_prob = base_fill_prob
        self.edge_sensitivity = edge_sensitivity
        self.rng = random.Random(random_seed)

    def simulate_fill(
        self,
        bid: float,
        market_ask: float,
        size: float,
    ) -> tuple[bool, float]:
        """Simulate whether a fill occurs.

        Args:
            bid: Our bid price
            market_ask: Market ask price
            size: Order size

        Returns:
            (filled, fill_qty) - filled is True if fill occurred
        """
        edge = market_ask - bid
        if edge <= 0:
            return False, 0.0

        # Fill probability increases with edge
        fill_prob = min(0.9, self.base_fill_prob * (1 + self.edge_sensitivity * edge))

        if self.rng.random() < fill_prob:
            # Partial fill simulation - fill between 50-100% of size
            fill_pct = 0.5 + 0.5 * self.rng.random()
            return True, size * fill_pct

        return False, 0.0


class Backtester:
    """Backtester for the InventoryMMQuoter.

    Simulates quoter performance against a series of market ticks.
    """

    def __init__(
        self,
        fill_simulator: FillSimulator | None = None,
        initial_inventory: Inventory | None = None,
    ) -> None:
        """Initialize backtester.

        Args:
            fill_simulator: Fill simulation model (default: FillSimulator())
            initial_inventory: Starting inventory (default: empty)
        """
        self.fill_simulator = fill_simulator or FillSimulator()
        self.initial_inventory = initial_inventory or Inventory()

    def run(
        self,
        quoter: InventoryMMQuoter,
        ticks: list[MarketTick],
    ) -> BacktestResult:
        """Run backtest on a series of market ticks.

        Args:
            quoter: The quoter to test
            ticks: Market data ticks

        Returns:
            BacktestResult with metrics and history
        """
        inventory = self.initial_inventory.model_copy()
        fills: list[FillRecord] = []
        pnl_history: list[float] = []
        inventory_history: list[tuple[float, float]] = []
        total_quotes = 0

        for tick in ticks:
            # Build market and oracle from tick
            market = Market(
                best_ask_up=tick.best_ask_up,
                best_bid_up=tick.best_bid_up,
                best_ask_down=tick.best_ask_down,
                best_bid_down=tick.best_bid_down,
            )
            oracle = Oracle(
                current_price=tick.oracle_price,
                threshold=tick.threshold,
            )

            # Generate quotes
            quote = quoter.quote(
                inventory=inventory,
                market=market,
                oracle=oracle,
                minutes_to_resolution=tick.minutes_to_resolution,
            )

            # Simulate fills for UP
            if quote.bid_up is not None:
                total_quotes += 1
                filled, qty = self.fill_simulator.simulate_fill(
                    quote.bid_up, market.best_ask_up, quote.size_up
                )
                if filled and qty > 0:
                    spread_captured = market.best_ask_up - quote.bid_up
                    fills.append(
                        FillRecord(
                            timestamp=tick.timestamp,
                            side="up",
                            qty=qty,
                            price=quote.bid_up,
                            spread_captured=spread_captured,
                        )
                    )
                    inventory = inventory.update_position("up", qty, quote.bid_up)

            # Simulate fills for DOWN
            if quote.bid_down is not None:
                total_quotes += 1
                filled, qty = self.fill_simulator.simulate_fill(
                    quote.bid_down, market.best_ask_down, quote.size_down
                )
                if filled and qty > 0:
                    spread_captured = market.best_ask_down - quote.bid_down
                    fills.append(
                        FillRecord(
                            timestamp=tick.timestamp,
                            side="down",
                            qty=qty,
                            price=quote.bid_down,
                            spread_captured=spread_captured,
                        )
                    )
                    inventory = inventory.update_position("down", qty, quote.bid_down)

            # Record state
            inventory_history.append((inventory.up_qty, inventory.down_qty))

            # Calculate current PnL (mark-to-market)
            mid_up = (market.best_ask_up + market.best_bid_up) / 2
            mid_down = (market.best_ask_down + market.best_bid_down) / 2
            pairs = inventory.pairs
            realized = pairs * (1.0 - inventory.combined_avg)
            unrealized = (
                (inventory.up_qty - pairs) * (mid_up - inventory.up_avg)
                + (inventory.down_qty - pairs) * (mid_down - inventory.down_avg)
            )
            pnl_history.append(realized + unrealized)

        # Get final market prices for metrics
        if ticks:
            final_tick = ticks[-1]
            final_up_mid = (final_tick.best_ask_up + final_tick.best_bid_up) / 2
            final_down_mid = (final_tick.best_ask_down + final_tick.best_bid_down) / 2
        else:
            final_up_mid = 0.5
            final_down_mid = 0.5

        # Convert fills to dict format for metrics
        fills_dict = [
            {
                "side": f.side,
                "qty": f.qty,
                "price": f.price,
                "spread_captured": f.spread_captured,
            }
            for f in fills
        ]

        metrics = calculate_metrics(
            pnl_history=pnl_history,
            fills=fills_dict,
            final_inventory_up=inventory.up_qty,
            final_inventory_down=inventory.down_qty,
            final_up_avg=inventory.up_avg,
            final_down_avg=inventory.down_avg,
            total_quotes=total_quotes,
            final_market_up_mid=final_up_mid,
            final_market_down_mid=final_down_mid,
        )

        return BacktestResult(
            metrics=metrics,
            fills=fills,
            pnl_history=pnl_history,
            inventory_history=inventory_history,
            params=quoter.params,
        )
