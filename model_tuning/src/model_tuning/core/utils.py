"""Utility functions for the quoter."""

from model_tuning.core.models import Market

# Polymarket only accepts prices in whole cents (0.01, 0.02, ... 0.99)
# Any price like 0.515 or 0.4875 is INVALID and will be rejected
TICK_SIZE = 0.01


def snap_to_tick(value: float) -> float:
    """Snap a price to the nearest valid Polymarket tick (whole cent).

    Polymarket only accepts prices in whole cents (0.01, 0.02, ... 0.99).
    Any price like 0.515 or 0.4875 is INVALID and will be rejected.

    Examples:
        snap_to_tick(0.515)  -> 0.52
        snap_to_tick(0.494)  -> 0.49
        snap_to_tick(0.4875) -> 0.49
    """
    return round(round(value / TICK_SIZE) * TICK_SIZE, 2)


def create_market(up_mid: float, spread: float = 0.02) -> Market:
    """Create a realistic COMPLEMENTARY orderbook.

    In Polymarket binary markets, UP + DOWN = $1.00 at resolution.
    Therefore orderbooks are complementary:
        UP_ask ~ 1 - DOWN_bid
        DOWN_ask ~ 1 - UP_bid

    This function ensures asks sum to approximately $1.00 + overround.

    Args:
        up_mid: Midpoint for UP probability (e.g., 0.55 means UP is 55% likely)
        spread: Bid-ask spread on each side (default 2c)

    Returns:
        Market with complementary prices

    Example:
        create_market(up_mid=0.55, spread=0.02)
        -> UP: bid=0.54, ask=0.56
        -> DOWN: bid=0.44, ask=0.46
        -> Asks sum to 1.02 (2% overround)
        -> Combined bid guaranteed < 1.00!
    """
    down_mid = 1.0 - up_mid
    return Market(
        best_ask_up=round(up_mid + spread / 2, 2),
        best_bid_up=round(up_mid - spread / 2, 2),
        best_ask_down=round(down_mid + spread / 2, 2),
        best_bid_down=round(down_mid - spread / 2, 2),
    )


def clamp(value: float, min_val: float, max_val: float) -> float:
    """Clamp a value to a range."""
    return max(min_val, min(max_val, value))
