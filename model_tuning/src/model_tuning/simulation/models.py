"""Data models for real-data simulation.

These models represent actual market data structures from Polymarket:
- Orderbook snapshots with full depth
- Real trade fills from the market
- Oracle price snapshots
"""

from typing import Literal

from pydantic import BaseModel, Field

from model_tuning.core.models import Inventory


class OrderbookLevel(BaseModel):
    """Single level in orderbook (price/size pair)."""

    price: float = Field(description="Price at this level")
    size: float = Field(description="Total size at this level")


class Orderbook(BaseModel):
    """Full orderbook for one side (UP or DOWN)."""

    asks: list[OrderbookLevel] = Field(default_factory=list)
    bids: list[OrderbookLevel] = Field(default_factory=list)

    @property
    def best_ask(self) -> float | None:
        """Best (lowest) ask price."""
        if not self.asks:
            return None
        return min(level.price for level in self.asks)

    @property
    def best_bid(self) -> float | None:
        """Best (highest) bid price."""
        if not self.bids:
            return None
        return max(level.price for level in self.bids)


class OrderbookSnapshot(BaseModel):
    """Combined orderbook snapshot for both UP and DOWN at a point in time."""

    up: Orderbook
    down: Orderbook
    timestamp: float = Field(description="Unix timestamp or relative time")


class RealFill(BaseModel):
    """A fill that occurred in the real market.

    Represents an actual trade execution that we'll use to determine
    if our quotes would have been filled.
    """

    price: float = Field(description="Price at which fill occurred")
    size: float = Field(description="Size of the fill")
    side: Literal["buy", "sell"] = Field(description="Trade side")
    timestamp: float = Field(description="Unix timestamp of fill")
    outcome: Literal["up", "down"] = Field(description="Which outcome this fill is for")


class OracleSnapshot(BaseModel):
    """Oracle price at a point in time.

    Contains both the oracle price and the threshold for the market question.
    """

    price: float = Field(description="Current oracle price (e.g., BTC price)")
    threshold: float = Field(description="Market question threshold")
    timestamp: float = Field(description="Unix timestamp")


class PositionState(BaseModel):
    """Position state at a point in time (for tracking history).

    Captures full inventory state plus computed metrics at each timestep.
    """

    timestamp: float
    up_qty: float
    down_qty: float
    up_avg: float
    down_avg: float
    pairs: float
    combined_avg: float
    potential_profit: float

    @classmethod
    def from_inventory(cls, inventory: Inventory, timestamp: float) -> "PositionState":
        """Create PositionState from Inventory at given timestamp."""
        return cls(
            timestamp=timestamp,
            up_qty=inventory.up_qty,
            down_qty=inventory.down_qty,
            up_avg=inventory.up_avg,
            down_avg=inventory.down_avg,
            pairs=inventory.pairs,
            combined_avg=inventory.combined_avg,
            potential_profit=inventory.potential_profit,
        )


class MatchedFill(BaseModel):
    """A fill that matched our quote.

    Records when a market fill hit our quoted bid, including
    both our bid price and the original market fill details.
    """

    timestamp: float = Field(description="Time of the fill")
    outcome: Literal["up", "down"] = Field(description="Which outcome filled")
    price: float = Field(description="Our bid price (what we paid)")
    size: float = Field(description="Size filled")
    original_fill: RealFill = Field(description="Reference to original market fill")


class OrderbookHistoryEntry(BaseModel):
    """Best ask prices at a point in time (for graphing)."""

    timestamp: float = Field(description="Unix timestamp")
    best_ask_up: float = Field(description="Best ask for UP token")
    best_ask_down: float = Field(description="Best ask for DOWN token")
