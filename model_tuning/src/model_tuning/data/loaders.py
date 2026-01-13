"""Data loading utilities for backtesting."""

import math
import random
from pathlib import Path

import pandas as pd

from model_tuning.tuning.backtester import MarketTick


def load_ticks_from_csv(path: str | Path) -> list[MarketTick]:
    """Load market ticks from CSV file.

    Expected columns:
        - timestamp: float (minutes from start or epoch)
        - oracle_price: float
        - threshold: float
        - best_ask_up, best_bid_up: float
        - best_ask_down, best_bid_down: float
        - minutes_to_resolution: float

    Args:
        path: Path to CSV file

    Returns:
        List of MarketTick objects
    """
    df = pd.read_csv(path)
    return _df_to_ticks(df)


def load_ticks_from_parquet(path: str | Path) -> list[MarketTick]:
    """Load market ticks from Parquet file.

    See load_ticks_from_csv for expected columns.

    Args:
        path: Path to Parquet file

    Returns:
        List of MarketTick objects
    """
    df = pd.read_parquet(path)
    return _df_to_ticks(df)


def _df_to_ticks(df: pd.DataFrame) -> list[MarketTick]:
    """Convert DataFrame to list of MarketTick."""
    required_cols = {
        "timestamp",
        "oracle_price",
        "threshold",
        "best_ask_up",
        "best_bid_up",
        "best_ask_down",
        "best_bid_down",
        "minutes_to_resolution",
    }

    missing = required_cols - set(df.columns)
    if missing:
        raise ValueError(f"Missing required columns: {missing}")

    ticks = []
    for _, row in df.iterrows():
        ticks.append(
            MarketTick(
                timestamp=float(row["timestamp"]),
                oracle_price=float(row["oracle_price"]),
                threshold=float(row["threshold"]),
                best_ask_up=float(row["best_ask_up"]),
                best_bid_up=float(row["best_bid_up"]),
                best_ask_down=float(row["best_ask_down"]),
                best_bid_down=float(row["best_bid_down"]),
                minutes_to_resolution=float(row["minutes_to_resolution"]),
            )
        )
    return ticks


def generate_synthetic_ticks(
    duration_minutes: float = 15.0,
    tick_interval_seconds: float = 5.0,
    threshold: float = 97000.0,
    initial_price: float = 97000.0,
    volatility: float = 0.0001,
    spread: float = 0.02,
    random_seed: int | None = None,
) -> list[MarketTick]:
    """Generate synthetic market ticks for testing.

    Simulates a 15-minute binary market with price following a random walk.

    Args:
        duration_minutes: Total duration in minutes
        tick_interval_seconds: Time between ticks in seconds
        threshold: Market question threshold (e.g., BTC > $97,000)
        initial_price: Starting oracle price
        volatility: Price volatility per tick (as fraction)
        spread: Bid-ask spread on each side
        random_seed: Random seed for reproducibility

    Returns:
        List of MarketTick objects
    """
    rng = random.Random(random_seed)
    ticks = []

    total_seconds = duration_minutes * 60
    num_ticks = int(total_seconds / tick_interval_seconds)

    price = initial_price

    for i in range(num_ticks):
        # Current time in minutes from end
        elapsed_seconds = i * tick_interval_seconds
        minutes_to_resolution = duration_minutes - (elapsed_seconds / 60)

        # Random walk for oracle price
        price_change = rng.gauss(0, price * volatility)
        price += price_change

        # Calculate fair value based on distance from threshold
        distance_pct = (price - threshold) / threshold

        # Simple fair value model: sigmoid of distance
        # When price >> threshold, UP approaches 1.0
        # When price << threshold, UP approaches 0.0
        fair_up = 1 / (1 + math.exp(-distance_pct * 200))  # Steep sigmoid
        fair_up = max(0.05, min(0.95, fair_up))  # Clamp to reasonable range

        fair_down = 1 - fair_up

        # Add some noise to fair value
        fair_up += rng.gauss(0, 0.01)
        fair_up = max(0.05, min(0.95, fair_up))
        fair_down = 1 - fair_up

        # Create bid/ask around fair value
        tick = MarketTick(
            timestamp=elapsed_seconds / 60,  # in minutes
            oracle_price=price,
            threshold=threshold,
            best_ask_up=round(fair_up + spread / 2, 2),
            best_bid_up=round(fair_up - spread / 2, 2),
            best_ask_down=round(fair_down + spread / 2, 2),
            best_bid_down=round(fair_down - spread / 2, 2),
            minutes_to_resolution=minutes_to_resolution,
        )
        ticks.append(tick)

    return ticks


def generate_trending_ticks(
    duration_minutes: float = 15.0,
    tick_interval_seconds: float = 5.0,
    threshold: float = 97000.0,
    initial_price: float = 97000.0,
    trend: float = 0.00005,
    volatility: float = 0.0001,
    spread: float = 0.02,
    random_seed: int | None = None,
) -> list[MarketTick]:
    """Generate synthetic ticks with a price trend.

    Similar to generate_synthetic_ticks but with drift.

    Args:
        duration_minutes: Total duration in minutes
        tick_interval_seconds: Time between ticks
        threshold: Market question threshold
        initial_price: Starting oracle price
        trend: Drift per tick (positive = bullish, negative = bearish)
        volatility: Random volatility per tick
        spread: Bid-ask spread
        random_seed: Random seed

    Returns:
        List of MarketTick objects
    """
    rng = random.Random(random_seed)
    ticks = []

    total_seconds = duration_minutes * 60
    num_ticks = int(total_seconds / tick_interval_seconds)

    price = initial_price

    for i in range(num_ticks):
        elapsed_seconds = i * tick_interval_seconds
        minutes_to_resolution = duration_minutes - (elapsed_seconds / 60)

        # Random walk with drift
        price_change = price * trend + rng.gauss(0, price * volatility)
        price += price_change

        # Fair value calculation
        distance_pct = (price - threshold) / threshold
        fair_up = 1 / (1 + math.exp(-distance_pct * 200))
        fair_up = max(0.05, min(0.95, fair_up))
        fair_down = 1 - fair_up

        tick = MarketTick(
            timestamp=elapsed_seconds / 60,
            oracle_price=price,
            threshold=threshold,
            best_ask_up=round(fair_up + spread / 2, 2),
            best_bid_up=round(fair_up - spread / 2, 2),
            best_ask_down=round(fair_down + spread / 2, 2),
            best_bid_down=round(fair_down - spread / 2, 2),
            minutes_to_resolution=minutes_to_resolution,
        )
        ticks.append(tick)

    return ticks
