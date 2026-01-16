"""Real-data simulator for the Polymarket quoter.

Replays historical orderbook data and fills against the quoter to evaluate
performance with actual market conditions.
"""

from bisect import bisect_left, bisect_right
from dataclasses import dataclass, field

from model_tuning.core.models import Inventory, Market, Oracle
from model_tuning.core.quoter import InventoryMMQuoter, QuoterParams
from model_tuning.simulation.models import (
    MatchedFill,
    OrderbookHistoryEntry,
    OrderbookSnapshot,
    OracleSnapshot,
    PositionState,
    RealFill,
)


@dataclass
class SimulationResult:
    """Results from a real-data simulation run."""

    final_inventory: Inventory
    """Final inventory state."""

    position_history: list[PositionState] = field(default_factory=list)
    """Position state at each orderbook snapshot."""

    matched_fills: list[MatchedFill] = field(default_factory=list)
    """All fills that matched our quotes."""

    orderbook_history: list[OrderbookHistoryEntry] = field(default_factory=list)
    """Best ask prices at each orderbook snapshot (for graphing)."""

    total_fills: int = 0
    """Total number of fills."""

    up_fills: int = 0
    """Number of UP fills."""

    down_fills: int = 0
    """Number of DOWN fills."""

    total_volume: float = 0.0
    """Total volume filled."""

    final_pnl_potential: float = 0.0
    """Potential profit if all pairs redeemed: pairs * (1 - combined_avg)."""

    params: QuoterParams | None = None
    """Parameters used for this simulation."""


class RealDataSimulator:
    """Simulates quoter against real orderbook data and fills.

    Key assumptions:
    1. We are always first in queue at our price level
    2. A fill matches our quote if:
       - Same outcome (up/down)
       - Fill is a BUY (someone buying = hitting our bid)
       - Fill price >= our bid price
    3. We can only fill up to our quoted size per timestep
    """

    def __init__(
        self,
        default_minutes_to_resolution: float = 10.0,
    ) -> None:
        """Initialize simulator.

        Args:
            default_minutes_to_resolution: Default time to resolution
                (used if not calculable from data)
        """
        self.default_minutes_to_resolution = default_minutes_to_resolution

    def _build_market(self, snapshot: OrderbookSnapshot) -> Market:
        """Build Market object from orderbook snapshot.

        Args:
            snapshot: Orderbook snapshot with UP and DOWN books

        Returns:
            Market object with best bid/ask for each side
        """
        # Get best prices, defaulting to 0.5 if no orders
        best_ask_up = snapshot.up.best_ask or 0.51
        best_bid_up = snapshot.up.best_bid or 0.49
        best_ask_down = snapshot.down.best_ask or 0.51
        best_bid_down = snapshot.down.best_bid or 0.49

        return Market(
            best_ask_up=best_ask_up,
            best_bid_up=best_bid_up,
            best_ask_down=best_ask_down,
            best_bid_down=best_bid_down,
        )

    def _build_oracle(
        self,
        timestamp: float,
        oracle_data: list[OracleSnapshot],
    ) -> Oracle:
        """Build Oracle object by finding the oracle snapshot at or before timestamp.

        Uses binary search to find the most recent oracle snapshot.

        Args:
            timestamp: Current simulation time
            oracle_data: List of oracle snapshots (sorted by timestamp)

        Returns:
            Oracle object with price and threshold
        """
        if not oracle_data:
            return Oracle(current_price=97000, threshold=97000)  # Neutral default

        # Binary search for the latest oracle snapshot at or before timestamp
        timestamps = [o.timestamp for o in oracle_data]
        idx = bisect_right(timestamps, timestamp) - 1

        if idx < 0:
            # Use first oracle if timestamp is before all oracle data
            snapshot = oracle_data[0]
        else:
            snapshot = oracle_data[idx]

        return Oracle(
            current_price=snapshot.price,
            threshold=snapshot.threshold,
        )

    def _get_fills_in_window(
        self,
        fills: list[RealFill],
        start_time: float,
        end_time: float,
    ) -> list[RealFill]:
        """Get fills that occurred between start_time and end_time.

        Uses binary search for efficiency on large fill lists.

        Args:
            fills: List of fills (sorted by timestamp)
            start_time: Start of window (inclusive)
            end_time: End of window (exclusive)

        Returns:
            List of fills in the time window
        """
        timestamps = [f.timestamp for f in fills]
        start_idx = bisect_left(timestamps, start_time)
        end_idx = bisect_left(timestamps, end_time)

        return fills[start_idx:end_idx]

    def _match_fills(
        self,
        fills_in_window: list[RealFill],
        bid_up: float | None,
        size_up: float,
        bid_down: float | None,
        size_down: float,
        window_timestamp: float,
    ) -> tuple[list[MatchedFill], float, float]:
        """Match fills against our quotes.

        Checks each fill to see if it would have filled our order:
        - Only SELL fills (someone selling hits our bid)
        - Fill price must be <= our bid (they sold at or below our bid)
        - We are assumed first in queue

        Args:
            fills_in_window: Fills to check
            bid_up: Our UP bid price (None = not quoting)
            size_up: Our UP quote size
            bid_down: Our DOWN bid price (None = not quoting)
            size_down: Our DOWN quote size
            window_timestamp: Timestamp for matched fills

        Returns:
            (matched_fills, filled_up_size, filled_down_size)
        """
        matched: list[MatchedFill] = []
        filled_up = 0.0
        filled_down = 0.0

        for fill in fills_in_window:
            # Only match SELL fills (someone selling = hitting our bid)
            if fill.side != "sell":
                continue

            if fill.outcome == "up" and bid_up is not None:
                # Check if fill price <= our bid (they sold at or below our bid)
                if fill.price <= bid_up:
                    # Fill up to remaining size
                    remaining = size_up - filled_up
                    if remaining > 0:
                        fill_size = min(fill.size, remaining)
                        matched.append(
                            MatchedFill(
                                timestamp=fill.timestamp,
                                outcome="up",
                                price=bid_up,  # We get our bid price
                                size=fill_size,
                                original_fill=fill,
                            )
                        )
                        filled_up += fill_size

            elif fill.outcome == "down" and bid_down is not None:
                if fill.price <= bid_down:
                    remaining = size_down - filled_down
                    if remaining > 0:
                        fill_size = min(fill.size, remaining)
                        matched.append(
                            MatchedFill(
                                timestamp=fill.timestamp,
                                outcome="down",
                                price=bid_down,
                                size=fill_size,
                                original_fill=fill,
                            )
                        )
                        filled_down += fill_size

        return matched, filled_up, filled_down

    def run(
        self,
        quoter: InventoryMMQuoter,
        orderbooks: list[OrderbookSnapshot],
        fills: list[RealFill],
        oracle: list[OracleSnapshot],
        initial_inventory: Inventory | None = None,
        resolution_timestamp: float | None = None,
    ) -> SimulationResult:
        """Run simulation against real data.

        Processes orderbook snapshots sequentially, generating quotes at each
        step and matching against fills that occurred before the next snapshot.

        Args:
            quoter: The quoter to test
            orderbooks: List of orderbook snapshots (sorted by timestamp)
            fills: List of fills (sorted by timestamp)
            oracle: List of oracle snapshots (sorted by timestamp)
            initial_inventory: Starting inventory (default: zero inventory)
            resolution_timestamp: Market resolution time (for minutes_to_resolution)

        Returns:
            SimulationResult with position history and fill details
        """
        # Initialize inventory with zero position
        if initial_inventory is None:
            inventory = Inventory(up_qty=0, down_qty=0, up_avg=0.5, down_avg=0.5)
        else:
            inventory = initial_inventory.model_copy()

        # Calculate resolution timestamp if not provided
        if resolution_timestamp is None and orderbooks:
            # Assume resolution is 15 minutes after last orderbook
            resolution_timestamp = orderbooks[-1].timestamp + 15 * 60

        position_history: list[PositionState] = []
        all_matched_fills: list[MatchedFill] = []
        orderbook_history: list[OrderbookHistoryEntry] = []

        for i, snapshot in enumerate(orderbooks):
            # Build Market and Oracle
            market = self._build_market(snapshot)

            # Record orderbook best asks for graphing
            orderbook_history.append(
                OrderbookHistoryEntry(
                    timestamp=snapshot.timestamp,
                    best_ask_up=snapshot.up.best_ask or 0.5,
                    best_ask_down=snapshot.down.best_ask or 0.5,
                )
            )
            oracle_obj = self._build_oracle(snapshot.timestamp, oracle)

            # Calculate minutes to resolution
            if resolution_timestamp:
                minutes_to_resolution = max(
                    0.0, (resolution_timestamp - snapshot.timestamp) / 60.0
                )
            else:
                minutes_to_resolution = self.default_minutes_to_resolution

            # Generate quotes
            quote = quoter.quote(
                inventory=inventory,
                market=market,
                oracle=oracle_obj,
                minutes_to_resolution=minutes_to_resolution,
            )

            # Determine time window for fills
            # Window is from current snapshot to next snapshot (or end)
            start_time = snapshot.timestamp
            if i + 1 < len(orderbooks):
                end_time = orderbooks[i + 1].timestamp
            else:
                end_time = snapshot.timestamp + 60  # 1 minute window for last snapshot

            # Get fills in this window
            fills_in_window = self._get_fills_in_window(fills, start_time, end_time)

            # Match fills against our quotes
            matched, filled_up, filled_down = self._match_fills(
                fills_in_window,
                quote.bid_up,
                quote.size_up,
                quote.bid_down,
                quote.size_down,
                snapshot.timestamp,
            )

            # Update inventory with fills
            for mf in matched:
                inventory = inventory.update_position(mf.outcome, mf.size, mf.price)
                all_matched_fills.append(mf)

            # Record position state
            position_history.append(
                PositionState.from_inventory(inventory, snapshot.timestamp)
            )

        # Calculate summary stats
        up_fills = sum(1 for mf in all_matched_fills if mf.outcome == "up")
        down_fills = sum(1 for mf in all_matched_fills if mf.outcome == "down")
        total_volume = sum(mf.size for mf in all_matched_fills)

        return SimulationResult(
            final_inventory=inventory,
            position_history=position_history,
            matched_fills=all_matched_fills,
            orderbook_history=orderbook_history,
            total_fills=len(all_matched_fills),
            up_fills=up_fills,
            down_fills=down_fills,
            total_volume=total_volume,
            final_pnl_potential=inventory.pairs * inventory.potential_profit,
            params=quoter.params,
        )
