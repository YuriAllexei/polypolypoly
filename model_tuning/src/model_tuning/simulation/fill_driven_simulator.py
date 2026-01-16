"""Fill-driven simulator for backtesting market making strategies.

Iterates over fills (not orderbook snapshots) for efficiency, with
on-demand orderbook reconstruction and full PnL tracking.
"""

from bisect import bisect_right
from dataclasses import dataclass, field

from model_tuning.core.models import Inventory
from model_tuning.simulation.models import (
    EnhancedPositionState,
    MatchedFill,
    OracleSnapshot,
    RealFill,
)
from model_tuning.simulation.orderbook_reconstructor import OrderbookReconstructor
from model_tuning.simulation.quoters import SimulationQuoter


@dataclass
class FillDrivenSimulationResult:
    """Results from a fill-driven simulation run."""

    final_inventory: Inventory
    """Final inventory state."""

    position_history: list[EnhancedPositionState] = field(default_factory=list)
    """Position state at each matched fill (with full PnL)."""

    matched_fills: list[MatchedFill] = field(default_factory=list)
    """All fills that matched our quotes."""

    oracle_history: list[OracleSnapshot] = field(default_factory=list)
    """Oracle snapshots used during simulation."""

    total_fills_considered: int = 0
    """Total number of SELL fills considered (excluding BUY fills)."""

    total_fills_matched: int = 0
    """Number of fills that matched our quotes."""

    up_fills: int = 0
    """Number of UP fills matched."""

    down_fills: int = 0
    """Number of DOWN fills matched."""

    total_volume: float = 0.0
    """Total volume filled (sum of all matched fill sizes)."""

    final_merged_pnl: float = 0.0
    """Final merged PnL: pairs * (1 - combined_avg)."""

    final_directional_pnl: float = 0.0
    """Final directional PnL: excess_qty * (market_price - avg_cost)."""

    final_total_pnl: float = 0.0
    """Final total PnL: merged + directional."""


class FillDrivenSimulator:
    """Fill-driven simulator that iterates over fills with on-demand orderbook reconstruction.

    Key differences from RealDataSimulator:
    1. Iterates fills (not orderbook snapshots) - much faster
    2. On-demand orderbook reconstruction - memory efficient
    3. Full PnL tracking (merged + directional)

    Assumptions:
    1. We are first in queue at our price level
    2. A fill matches our quote if:
       - Fill is a SELL (someone selling = hitting our bid)
       - fill.price <= our_bid (they sold at or below our bid)
    3. We fill the entire fill size (no partial fills based on our quote size)
    """

    def __init__(self) -> None:
        """Initialize the fill-driven simulator."""
        pass

    def _get_oracle_at(
        self,
        timestamp: float,
        oracle_data: list[OracleSnapshot],
    ) -> OracleSnapshot | None:
        """Get oracle snapshot at or before timestamp using binary search.

        Args:
            timestamp: Target timestamp
            oracle_data: List of oracle snapshots (sorted by timestamp)

        Returns:
            OracleSnapshot at or before timestamp, or None if no oracle data
        """
        if not oracle_data:
            return None

        timestamps = [o.timestamp for o in oracle_data]
        idx = bisect_right(timestamps, timestamp) - 1

        if idx < 0:
            # Timestamp is before all oracle data, use first
            return oracle_data[0]

        return oracle_data[idx]

    def run(
        self,
        quoter: SimulationQuoter,
        reconstructor: OrderbookReconstructor,
        fills: list[RealFill],
        oracle: list[OracleSnapshot],
        initial_inventory: Inventory | None = None,
    ) -> FillDrivenSimulationResult:
        """Run fill-driven simulation.

        Iterates over fills chronologically, reconstructing orderbook state
        on-demand and matching fills against our quotes.

        Args:
            quoter: Quoter that generates bids based on orderbook state
            reconstructor: On-demand orderbook reconstructor
            fills: List of fills (sorted by timestamp)
            oracle: List of oracle snapshots (sorted by timestamp)
            initial_inventory: Starting inventory (default: zero inventory)

        Returns:
            FillDrivenSimulationResult with position history and fill details
        """
        # Initialize inventory
        if initial_inventory is None:
            inventory = Inventory(up_qty=0, down_qty=0, up_avg=0.5, down_avg=0.5)
        else:
            inventory = initial_inventory.model_copy()

        position_history: list[EnhancedPositionState] = []
        matched_fills: list[MatchedFill] = []
        oracle_history: list[OracleSnapshot] = []

        total_fills_considered = 0
        up_fills = 0
        down_fills = 0
        total_volume = 0.0

        for fill in fills:
            # Only SELL fills hit our bids
            # (someone selling = we're buying from them)
            if fill.side != "sell":
                continue

            total_fills_considered += 1

            # 1. Reconstruct orderbook just before fill
            # Use timestamp - small epsilon to get state before the fill
            orderbook = reconstructor.get_orderbook_at(fill.timestamp - 0.001)

            # 2. Get oracle at fill time
            oracle_snapshot = self._get_oracle_at(fill.timestamp, oracle)
            if oracle_snapshot and (
                not oracle_history or oracle_history[-1] != oracle_snapshot
            ):
                oracle_history.append(oracle_snapshot)

            # 3. Generate quote
            quote = quoter.quote(orderbook, oracle_snapshot)

            # 4. Check match and update inventory
            matched = False

            if fill.outcome == "up" and quote.bid_up is not None:
                # Check if fill price <= our bid (they sold at or below our bid)
                if fill.price <= quote.bid_up:
                    # Match! Update inventory at OUR bid price
                    inventory = inventory.update_position("up", fill.size, quote.bid_up)
                    matched_fills.append(
                        MatchedFill(
                            timestamp=fill.timestamp,
                            outcome="up",
                            price=quote.bid_up,
                            size=fill.size,
                            original_fill=fill,
                        )
                    )
                    up_fills += 1
                    total_volume += fill.size
                    matched = True

            elif fill.outcome == "down" and quote.bid_down is not None:
                if fill.price <= quote.bid_down:
                    inventory = inventory.update_position(
                        "down", fill.size, quote.bid_down
                    )
                    matched_fills.append(
                        MatchedFill(
                            timestamp=fill.timestamp,
                            outcome="down",
                            price=quote.bid_down,
                            size=fill.size,
                            original_fill=fill,
                        )
                    )
                    down_fills += 1
                    total_volume += fill.size
                    matched = True

            # 5. Record position state with PnL (only on matched fills)
            if matched:
                # Reconstruct orderbook again for current market prices
                # (used for directional PnL mark-to-market)
                current_orderbook = reconstructor.get_orderbook_at(fill.timestamp)
                position_history.append(
                    EnhancedPositionState.from_inventory_and_orderbook(
                        inventory, current_orderbook, fill.timestamp
                    )
                )

        # Calculate final PnL
        final_merged_pnl = 0.0
        final_directional_pnl = 0.0
        final_total_pnl = 0.0

        if position_history:
            final_state = position_history[-1]
            final_merged_pnl = final_state.merged_pnl
            final_directional_pnl = final_state.directional_pnl
            final_total_pnl = final_state.total_pnl

        return FillDrivenSimulationResult(
            final_inventory=inventory,
            position_history=position_history,
            matched_fills=matched_fills,
            oracle_history=oracle_history,
            total_fills_considered=total_fills_considered,
            total_fills_matched=len(matched_fills),
            up_fills=up_fills,
            down_fills=down_fills,
            total_volume=total_volume,
            final_merged_pnl=final_merged_pnl,
            final_directional_pnl=final_directional_pnl,
            final_total_pnl=final_total_pnl,
        )
