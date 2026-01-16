"""Data loaders for real-data simulation.

Load orderbooks, fills, and oracle data from JSON files.
"""

import json
from pathlib import Path

from model_tuning.simulation.models import (
    Orderbook,
    OrderbookLevel,
    OrderbookSnapshot,
    OracleSnapshot,
    RealFill,
)


def load_orderbooks_from_json(path: str | Path) -> list[OrderbookSnapshot]:
    """Load orderbook snapshots from JSON file.

    Expected format:
    [
        {
            "up": {"asks": [{"price": 0.55, "size": 100}, ...], "bids": [...]},
            "down": {"asks": [...], "bids": [...]},
            "timestamp": 1704067200.0
        },
        ...
    ]

    Args:
        path: Path to JSON file

    Returns:
        List of OrderbookSnapshot sorted by timestamp
    """
    with open(path) as f:
        data = json.load(f)

    snapshots = []
    for item in data:
        # Parse UP orderbook
        up_asks = [OrderbookLevel(**level) for level in item["up"].get("asks", [])]
        up_bids = [OrderbookLevel(**level) for level in item["up"].get("bids", [])]
        up_book = Orderbook(asks=up_asks, bids=up_bids)

        # Parse DOWN orderbook
        down_asks = [OrderbookLevel(**level) for level in item["down"].get("asks", [])]
        down_bids = [OrderbookLevel(**level) for level in item["down"].get("bids", [])]
        down_book = Orderbook(asks=down_asks, bids=down_bids)

        snapshots.append(
            OrderbookSnapshot(
                up=up_book,
                down=down_book,
                timestamp=item["timestamp"],
            )
        )

    # Sort by timestamp
    return sorted(snapshots, key=lambda x: x.timestamp)


def load_fills_from_json(path: str | Path) -> list[RealFill]:
    """Load fills from JSON file.

    Expected format:
    [
        {"price": 0.55, "size": 100, "side": "buy", "timestamp": ..., "outcome": "up"},
        ...
    ]

    Args:
        path: Path to JSON file

    Returns:
        List of RealFill sorted by timestamp
    """
    with open(path) as f:
        data = json.load(f)

    fills = [RealFill(**item) for item in data]
    return sorted(fills, key=lambda x: x.timestamp)


def load_oracle_from_json(path: str | Path) -> list[OracleSnapshot]:
    """Load oracle snapshots from JSON file.

    Expected format:
    [
        {"price": 97500.0, "threshold": 98000.0, "timestamp": ...},
        ...
    ]

    Args:
        path: Path to JSON file

    Returns:
        List of OracleSnapshot sorted by timestamp
    """
    with open(path) as f:
        data = json.load(f)

    snapshots = [OracleSnapshot(**item) for item in data]
    return sorted(snapshots, key=lambda x: x.timestamp)


def load_orderbooks_from_raw(path: str | Path) -> list[OrderbookSnapshot]:
    """Load raw orderbook data (initial + deltas) and reconstruct snapshots.

    This function loads the raw orderbook data saved by the DataFetcher,
    which stores initial snapshots plus incremental price_changes to minimize
    storage for high-frequency orderbook updates.

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
        path: Path to orderbooks_raw.json file

    Returns:
        List of OrderbookSnapshot sorted by timestamp
    """
    with open(path) as f:
        data = json.load(f)

    up_token_id = data["up_token_id"]
    down_token_id = data["down_token_id"]
    initial_snapshots = data["initial_snapshots"]
    price_changes = data["price_changes"]

    # Build internal orderbook state (price -> size dicts)
    # Using strings as keys for exact price matching
    up_bids: dict[str, float] = {}
    up_asks: dict[str, float] = {}
    down_bids: dict[str, float] = {}
    down_asks: dict[str, float] = {}

    # Initialize from initial snapshots
    initial_timestamp = 0
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

    def build_snapshot(timestamp: float) -> OrderbookSnapshot:
        """Build OrderbookSnapshot from current state."""
        up_book = Orderbook(
            bids=[
                OrderbookLevel(price=float(p), size=s)
                for p, s in up_bids.items()
                if s > 0
            ],
            asks=[
                OrderbookLevel(price=float(p), size=s)
                for p, s in up_asks.items()
                if s > 0
            ],
        )
        down_book = Orderbook(
            bids=[
                OrderbookLevel(price=float(p), size=s)
                for p, s in down_bids.items()
                if s > 0
            ],
            asks=[
                OrderbookLevel(price=float(p), size=s)
                for p, s in down_asks.items()
                if s > 0
            ],
        )
        return OrderbookSnapshot(up=up_book, down=down_book, timestamp=timestamp)

    snapshots: list[OrderbookSnapshot] = []

    # Add initial snapshot
    if initial_timestamp > 0:
        snapshots.append(build_snapshot(initial_timestamp))

    # Group price_changes by timestamp and apply
    if not price_changes:
        return snapshots

    # Sort by timestamp to ensure chronological order
    sorted_changes = sorted(price_changes, key=lambda x: x["timestamp"])

    current_timestamp: float | None = None
    for change in sorted_changes:
        timestamp = change["timestamp"]
        asset_id = change["asset_id"]
        price_str = str(change["price"])
        size = change["size"]
        side = change["side"].lower()

        # Apply the change to appropriate orderbook
        if asset_id == up_token_id:
            if side == "buy":
                if size > 0:
                    up_bids[price_str] = size
                else:
                    up_bids.pop(price_str, None)
            else:  # sell
                if size > 0:
                    up_asks[price_str] = size
                else:
                    up_asks.pop(price_str, None)
        elif asset_id == down_token_id:
            if side == "buy":
                if size > 0:
                    down_bids[price_str] = size
                else:
                    down_bids.pop(price_str, None)
            else:  # sell
                if size > 0:
                    down_asks[price_str] = size
                else:
                    down_asks.pop(price_str, None)

        # Emit snapshot at each unique timestamp
        if current_timestamp is None or timestamp != current_timestamp:
            if current_timestamp is not None:
                # Emit snapshot for previous timestamp batch
                snapshots.append(build_snapshot(current_timestamp))
            current_timestamp = timestamp

    # Emit final snapshot
    if current_timestamp is not None:
        snapshots.append(build_snapshot(current_timestamp))

    return sorted(snapshots, key=lambda x: x.timestamp)


def load_simulation_data(
    orderbooks_path: str | Path,
    fills_path: str | Path,
    oracle_path: str | Path,
) -> tuple[list[OrderbookSnapshot], list[RealFill], list[OracleSnapshot]]:
    """Load all simulation data from JSON files.

    Convenience function to load all three data sources at once.

    Args:
        orderbooks_path: Path to orderbooks JSON
        fills_path: Path to fills JSON
        oracle_path: Path to oracle JSON

    Returns:
        (orderbooks, fills, oracle) tuple, all sorted by timestamp
    """
    orderbooks = load_orderbooks_from_json(orderbooks_path)
    fills = load_fills_from_json(fills_path)
    oracle = load_oracle_from_json(oracle_path)

    return orderbooks, fills, oracle


def load_simulation_data_from_raw(
    data_dir: str | Path,
) -> tuple[list[OrderbookSnapshot], list[RealFill], list[OracleSnapshot]]:
    """Load simulation data from raw format (as saved by DataFetcher).

    This function loads data from a directory containing:
    - fills.json: Fill data
    - oracle.json: Oracle price data
    - orderbooks_raw.json: Initial orderbook snapshots + price deltas

    Args:
        data_dir: Path to directory containing the data files (e.g., sim_data/<slug>/)

    Returns:
        (orderbooks, fills, oracle) tuple, all sorted by timestamp
    """
    data_dir = Path(data_dir)

    fills_path = data_dir / "fills.json"
    oracle_path = data_dir / "oracle.json"
    orderbooks_raw_path = data_dir / "orderbooks_raw.json"

    # Load fills and oracle (same format as before)
    fills = load_fills_from_json(fills_path) if fills_path.exists() else []
    oracle = load_oracle_from_json(oracle_path) if oracle_path.exists() else []

    # Load orderbooks from raw format (initial + deltas)
    orderbooks = (
        load_orderbooks_from_raw(orderbooks_raw_path)
        if orderbooks_raw_path.exists()
        else []
    )

    return orderbooks, fills, oracle
