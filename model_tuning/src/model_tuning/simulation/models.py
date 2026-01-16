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


class EnhancedPositionState(BaseModel):
    """Position state with full PnL tracking (merged + directional).

    Extends PositionState with:
    - Merged PnL: profit from balanced pairs
    - Directional PnL: mark-to-market value of excess inventory
    """

    # Basic position fields
    timestamp: float
    up_qty: float
    down_qty: float
    up_avg: float
    down_avg: float
    pairs: float
    combined_avg: float
    potential_profit: float

    # PnL fields
    merged_pnl: float = Field(description="pairs * (1 - combined_avg)")
    directional_qty: float = Field(description="abs(up_qty - down_qty)")
    excess_side: Literal["up", "down", "balanced"] = Field(
        description="Which side has excess inventory"
    )
    directional_market_price: float = Field(
        description="Best bid for the excess side (mark-to-market price)"
    )
    directional_avg_cost: float = Field(
        description="Average cost of the excess side"
    )
    directional_pnl: float = Field(
        description="directional_qty * (market_price - avg_cost)"
    )

    @property
    def total_pnl(self) -> float:
        """Total PnL = merged + directional."""
        return self.merged_pnl + self.directional_pnl

    @property
    def net_qty(self) -> float:
        """Net quantity = up_qty - down_qty (positive means long UP)."""
        return self.up_qty - self.down_qty

    @classmethod
    def from_inventory_and_orderbook(
        cls,
        inventory: Inventory,
        orderbook: "OrderbookSnapshot",
        timestamp: float,
    ) -> "EnhancedPositionState":
        """Create EnhancedPositionState from Inventory and orderbook.

        Args:
            inventory: Current inventory state
            orderbook: Current orderbook (for mark-to-market prices)
            timestamp: Current timestamp

        Returns:
            EnhancedPositionState with full PnL calculations
        """
        # Basic fields from inventory
        pairs = inventory.pairs
        combined_avg = inventory.combined_avg
        potential_profit = inventory.potential_profit

        # Merged PnL: profit from balanced pairs
        merged_pnl = pairs * (1.0 - combined_avg)

        # Directional position
        directional_qty = abs(inventory.up_qty - inventory.down_qty)

        if inventory.up_qty > inventory.down_qty:
            excess_side: Literal["up", "down", "balanced"] = "up"
            directional_market_price = orderbook.up.best_bid or 0.0
            directional_avg_cost = inventory.up_avg
        elif inventory.down_qty > inventory.up_qty:
            excess_side = "down"
            directional_market_price = orderbook.down.best_bid or 0.0
            directional_avg_cost = inventory.down_avg
        else:
            excess_side = "balanced"
            directional_market_price = 0.0
            directional_avg_cost = 0.0

        # Directional PnL: mark-to-market value of excess
        if directional_qty > 0:
            directional_pnl = directional_qty * (
                directional_market_price - directional_avg_cost
            )
        else:
            directional_pnl = 0.0

        return cls(
            timestamp=timestamp,
            up_qty=inventory.up_qty,
            down_qty=inventory.down_qty,
            up_avg=inventory.up_avg,
            down_avg=inventory.down_avg,
            pairs=pairs,
            combined_avg=combined_avg,
            potential_profit=potential_profit,
            merged_pnl=merged_pnl,
            directional_qty=directional_qty,
            excess_side=excess_side,
            directional_market_price=directional_market_price,
            directional_avg_cost=directional_avg_cost,
            directional_pnl=directional_pnl,
        )
