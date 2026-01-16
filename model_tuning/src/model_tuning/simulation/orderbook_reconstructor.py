"""On-demand orderbook reconstruction from raw delta format.

Provides memory-efficient orderbook reconstruction by applying deltas
incrementally as needed, rather than materializing all snapshots upfront.
"""

from bisect import bisect_right
from dataclasses import dataclass, field
from pathlib import Path
import json

from model_tuning.simulation.models import (
    Orderbook,
    OrderbookLevel,
    OrderbookSnapshot,
)


@dataclass
class OrderbookReconstructor:
    """On-demand orderbook reconstruction from initial state + deltas.

    Maintains internal orderbook state and applies deltas incrementally.
    Optimized for forward-only traversal (fills are chronological).

    Key features:
    - String keys for prices (avoids float comparison issues)
    - Binary search on pre-computed timestamp list
    - Forward-only: each delta applied exactly once -> O(n) total
    """

    up_token_id: str
    down_token_id: str

    # Internal state: price (str) -> size
    _up_bids: dict[str, float] = field(default_factory=dict)
    _up_asks: dict[str, float] = field(default_factory=dict)
    _down_bids: dict[str, float] = field(default_factory=dict)
    _down_asks: dict[str, float] = field(default_factory=dict)

    # Delta tracking
    _price_changes: list[dict] = field(default_factory=list)
    _change_timestamps: list[float] = field(default_factory=list)
    _last_processed_idx: int = -1

    # Initial timestamp
    _initial_timestamp: float = 0.0

    @classmethod
    def from_raw_data(cls, raw_data: dict) -> "OrderbookReconstructor":
        """Load from orderbooks_raw.json format.

        Expected format:
        {
            "up_token_id": "<token>",
            "down_token_id": "<token>",
            "initial_snapshots": {
                "<up_token>": {"timestamp": ..., "bids": [...], "asks": [...]},
                "<down_token>": {"timestamp": ..., "bids": [...], "asks": [...]}
            },
            "price_changes": [
                {"timestamp": ..., "asset_id": ..., "price": ..., "size": ..., "side": ...},
                ...
            ]
        }

        Args:
            raw_data: Dictionary loaded from orderbooks_raw.json

        Returns:
            Initialized OrderbookReconstructor
        """
        up_token_id = raw_data["up_token_id"]
        down_token_id = raw_data["down_token_id"]
        initial_snapshots = raw_data["initial_snapshots"]
        price_changes = raw_data.get("price_changes", [])

        # Initialize internal state from initial snapshots
        up_bids: dict[str, float] = {}
        up_asks: dict[str, float] = {}
        down_bids: dict[str, float] = {}
        down_asks: dict[str, float] = {}
        initial_timestamp = 0.0

        for token_id, snapshot in initial_snapshots.items():
            initial_timestamp = max(initial_timestamp, snapshot["timestamp"])
            if token_id == up_token_id:
                for level in snapshot.get("bids", []):
                    up_bids[str(level["price"])] = level["size"]
                for level in snapshot.get("asks", []):
                    up_asks[str(level["price"])] = level["size"]
            elif token_id == down_token_id:
                for level in snapshot.get("bids", []):
                    down_bids[str(level["price"])] = level["size"]
                for level in snapshot.get("asks", []):
                    down_asks[str(level["price"])] = level["size"]

        # Sort price changes by timestamp
        sorted_changes = sorted(price_changes, key=lambda x: x["timestamp"])

        # Pre-compute timestamp list for binary search
        change_timestamps = [c["timestamp"] for c in sorted_changes]

        return cls(
            up_token_id=up_token_id,
            down_token_id=down_token_id,
            _up_bids=up_bids,
            _up_asks=up_asks,
            _down_bids=down_bids,
            _down_asks=down_asks,
            _price_changes=sorted_changes,
            _change_timestamps=change_timestamps,
            _last_processed_idx=-1,
            _initial_timestamp=initial_timestamp,
        )

    @classmethod
    def from_file(cls, path: str | Path) -> "OrderbookReconstructor":
        """Load from orderbooks_raw.json file.

        Args:
            path: Path to orderbooks_raw.json

        Returns:
            Initialized OrderbookReconstructor
        """
        with open(path) as f:
            raw_data = json.load(f)
        return cls.from_raw_data(raw_data)

    def _apply_change(self, change: dict) -> None:
        """Apply a single price change to internal state.

        Args:
            change: Price change dict with asset_id, price, size, side
        """
        asset_id = change["asset_id"]
        price_str = str(change["price"])
        size = change["size"]
        side = change["side"].lower()

        if asset_id == self.up_token_id:
            if side == "buy":
                if size > 0:
                    self._up_bids[price_str] = size
                else:
                    self._up_bids.pop(price_str, None)
            else:  # sell
                if size > 0:
                    self._up_asks[price_str] = size
                else:
                    self._up_asks.pop(price_str, None)
        elif asset_id == self.down_token_id:
            if side == "buy":
                if size > 0:
                    self._down_bids[price_str] = size
                else:
                    self._down_bids.pop(price_str, None)
            else:  # sell
                if size > 0:
                    self._down_asks[price_str] = size
                else:
                    self._down_asks.pop(price_str, None)

    def _build_snapshot(self, timestamp: float) -> OrderbookSnapshot:
        """Build OrderbookSnapshot from current internal state.

        Args:
            timestamp: Timestamp for the snapshot

        Returns:
            OrderbookSnapshot with current state
        """
        up_book = Orderbook(
            bids=[
                OrderbookLevel(price=float(p), size=s)
                for p, s in self._up_bids.items()
                if s > 0
            ],
            asks=[
                OrderbookLevel(price=float(p), size=s)
                for p, s in self._up_asks.items()
                if s > 0
            ],
        )
        down_book = Orderbook(
            bids=[
                OrderbookLevel(price=float(p), size=s)
                for p, s in self._down_bids.items()
                if s > 0
            ],
            asks=[
                OrderbookLevel(price=float(p), size=s)
                for p, s in self._down_asks.items()
                if s > 0
            ],
        )
        return OrderbookSnapshot(up=up_book, down=down_book, timestamp=timestamp)

    def get_orderbook_at(self, timestamp: float) -> OrderbookSnapshot:
        """Get orderbook state at a specific timestamp.

        Applies deltas incrementally up to the given timestamp.
        Forward-only: assumes timestamps are requested in chronological order.

        Args:
            timestamp: Target timestamp

        Returns:
            OrderbookSnapshot at (or just before) the timestamp
        """
        if not self._change_timestamps:
            # No changes, return initial state
            return self._build_snapshot(timestamp)

        # Find the index of the last change at or before timestamp
        # bisect_right returns insertion point, so subtract 1 to get last change <= timestamp
        target_idx = bisect_right(self._change_timestamps, timestamp) - 1

        # Apply all changes from last_processed_idx+1 to target_idx (inclusive)
        for idx in range(self._last_processed_idx + 1, target_idx + 1):
            self._apply_change(self._price_changes[idx])

        self._last_processed_idx = max(self._last_processed_idx, target_idx)

        return self._build_snapshot(timestamp)

    def reset(self, raw_data: dict | None = None) -> None:
        """Reset to initial state for re-processing.

        Args:
            raw_data: If provided, reinitialize from this data.
                     Otherwise, cannot reset (would need original data).
        """
        if raw_data is None:
            raise ValueError(
                "Cannot reset without raw_data. "
                "Create a new instance with from_raw_data() instead."
            )

        new_instance = self.from_raw_data(raw_data)
        self._up_bids = new_instance._up_bids
        self._up_asks = new_instance._up_asks
        self._down_bids = new_instance._down_bids
        self._down_asks = new_instance._down_asks
        self._price_changes = new_instance._price_changes
        self._change_timestamps = new_instance._change_timestamps
        self._last_processed_idx = -1

    @property
    def initial_timestamp(self) -> float:
        """Get the timestamp of the initial snapshot."""
        return self._initial_timestamp

    @property
    def final_timestamp(self) -> float:
        """Get the timestamp of the last price change."""
        if self._change_timestamps:
            return self._change_timestamps[-1]
        return self._initial_timestamp
