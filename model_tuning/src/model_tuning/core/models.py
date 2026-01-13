"""Data models for the market-making quoter.

In Polymarket binary markets:
- Two complementary assets exist: UP and DOWN
- At resolution: UP + DOWN = $1.00 (one pays $1, other pays $0)
- Profit = $1.00 - (avg_cost_up + avg_cost_down) when holding both sides
"""

from pydantic import BaseModel, Field, computed_field


class Inventory(BaseModel):
    """Current position in UP and DOWN tokens.

    In binary markets, profit comes from holding BOTH sides:
    - If you hold 100 UP @ 48c and 100 DOWN @ 48c:
      - Combined cost = 48c + 48c = 96c per pair
      - At resolution, one side pays $1, other pays $0
      - But you have PAIRS, so you get $1 per pair
      - Profit = $1.00 - $0.96 = 4c per pair = $4.00 total
    """

    up_qty: float = Field(default=0.0, description="Number of UP tokens held")
    up_avg: float = Field(default=0.5, description="Average cost per UP token (e.g., 0.48 = 48c)")
    down_qty: float = Field(default=0.0, description="Number of DOWN tokens held")
    down_avg: float = Field(default=0.5, description="Average cost per DOWN token")

    @computed_field  # type: ignore[prop-decorator]
    @property
    def combined_avg(self) -> float:
        """Total cost per pair = up_avg + down_avg.

        This is THE KEY METRIC for profitability:
        - If combined_avg < 1.00: We profit on matched pairs
        - If combined_avg > 1.00: We lose on matched pairs (UNDERWATER)
        - If combined_avg = 1.00: Breakeven
        """
        return self.up_avg + self.down_avg

    @computed_field  # type: ignore[prop-decorator]
    @property
    def imbalance(self) -> float:
        """Normalized inventory imbalance: ranges from -1 to +1.

        Formula: q = (UP_qty - DOWN_qty) / (UP_qty + DOWN_qty)

        Interpretation:
            q = +1.0: 100% UP, 0% DOWN (extreme overweight UP)
            q = +0.5: 75% UP, 25% DOWN (overweight UP)
            q =  0.0: 50% UP, 50% DOWN (perfectly balanced)
            q = -0.5: 25% UP, 75% DOWN (overweight DOWN)
            q = -1.0: 0% UP, 100% DOWN (extreme overweight DOWN)

        WHY IT MATTERS:
            - Unmatched inventory is RISKY - you're exposed to direction
            - If overweight UP and DOWN wins, unmatched UP tokens = $0
            - Goal is to stay balanced (q ~ 0) so you can merge everything
        """
        total = self.up_qty + self.down_qty
        if total == 0:
            return 0.0
        return (self.up_qty - self.down_qty) / total

    @computed_field  # type: ignore[prop-decorator]
    @property
    def pairs(self) -> float:
        """Number of redeemable pairs = min(up_qty, down_qty).

        A "pair" is one UP + one DOWN token. At resolution:
        - Pair pays out $1.00 regardless of outcome
        - Your profit = $1.00 - combined_avg per pair
        """
        return min(self.up_qty, self.down_qty)

    @computed_field  # type: ignore[prop-decorator]
    @property
    def potential_profit(self) -> float:
        """Profit per pair if redeemed = 1.00 - combined_avg."""
        return 1.0 - self.combined_avg

    def update_position(
        self, side: str, qty: float, price: float
    ) -> "Inventory":
        """Return new Inventory after a fill.

        Args:
            side: "up" or "down"
            qty: Quantity filled
            price: Fill price

        Returns:
            New Inventory with updated position
        """
        if side == "up":
            new_qty = self.up_qty + qty
            new_avg = (
                (self.up_qty * self.up_avg + qty * price) / new_qty
                if new_qty > 0
                else self.up_avg
            )
            return Inventory(
                up_qty=new_qty,
                up_avg=new_avg,
                down_qty=self.down_qty,
                down_avg=self.down_avg,
            )
        else:
            new_qty = self.down_qty + qty
            new_avg = (
                (self.down_qty * self.down_avg + qty * price) / new_qty
                if new_qty > 0
                else self.down_avg
            )
            return Inventory(
                up_qty=self.up_qty,
                up_avg=self.up_avg,
                down_qty=new_qty,
                down_avg=new_avg,
            )


class Market(BaseModel):
    """Current market state from Polymarket orderbook.

    In a binary market, the asks should roughly sum to > $1.00 (overround)
    and bids should roughly sum to < $1.00 (underround).
    This spread is where market makers extract profit.
    """

    best_ask_up: float = Field(description="Cheapest price to BUY UP (we bid below this)")
    best_bid_up: float = Field(description="Best price someone will PAY for UP")
    best_ask_down: float = Field(description="Cheapest price to BUY DOWN")
    best_bid_down: float = Field(description="Best price someone will PAY for DOWN")

    @computed_field  # type: ignore[prop-decorator]
    @property
    def overround(self) -> float:
        """Ask overround: how much > $1.00 the asks sum to."""
        return self.best_ask_up + self.best_ask_down - 1.0

    @computed_field  # type: ignore[prop-decorator]
    @property
    def underround(self) -> float:
        """Bid underround: how much < $1.00 the bids sum to."""
        return 1.0 - (self.best_bid_up + self.best_bid_down)


class Oracle(BaseModel):
    """External price oracle (e.g., BTC price from exchange WebSocket).

    The oracle gives us an information edge:
    - If price > threshold: UP more likely to win
    - If price < threshold: DOWN more likely to win
    - How far above/below indicates confidence level
    """

    current_price: float = Field(description="Current price from exchange (e.g., 97200)")
    threshold: float = Field(description="The market question threshold (e.g., 97000)")

    @computed_field  # type: ignore[prop-decorator]
    @property
    def distance_pct(self) -> float:
        """How far is current price from threshold, as a percentage.

        Formula: (current - threshold) / threshold

        Examples:
            BTC=97500, threshold=97000 -> +0.52% (above threshold)
            BTC=96500, threshold=97000 -> -0.52% (below threshold)
            BTC=97000, threshold=97000 ->  0.00% (exactly at threshold)
        """
        return (self.current_price - self.threshold) / self.threshold

    @computed_field  # type: ignore[prop-decorator]
    @property
    def direction(self) -> str:
        """Human-readable direction."""
        if self.current_price > self.threshold:
            return "ABOVE"
        elif self.current_price < self.threshold:
            return "BELOW"
        return "AT"


class QuoteResult(BaseModel):
    """Output from the quoter - contains quotes and ALL diagnostic information.

    Tracks intermediate calculations from each layer for debugging.
    If bid_up or bid_down is None, we're NOT quoting that side.
    """

    # Final quotes
    bid_up: float | None = Field(default=None, description="Final UP bid (None = skip)")
    size_up: float = Field(default=0.0, description="Final UP size")
    bid_down: float | None = Field(default=None, description="Final DOWN bid (None = skip)")
    size_down: float = Field(default=0.0, description="Final DOWN size")

    # Layer 1: Oracle-Adjusted Offset
    oracle_adj: float = Field(description="Oracle adjustment: distance_pct x sensitivity")
    raw_up_offset: float = Field(description="UP offset BEFORE inventory skew")
    raw_down_offset: float = Field(description="DOWN offset BEFORE inventory skew")

    # Layer 2: Adverse Selection
    p_informed: float = Field(description="Probability of informed trade")
    base_spread: float = Field(description="Base spread (includes adverse selection)")

    # Layer 3: Inventory Skew
    inventory_q: float = Field(description="Imbalance: (UP - DOWN) / (UP + DOWN)")
    spread_mult_up: float = Field(description="Offset multiplier for UP (>1 if overweight UP)")
    spread_mult_down: float = Field(description="Offset multiplier for DOWN (<1 if overweight UP)")
    final_up_offset: float = Field(description="UP offset AFTER inventory skew")
    final_down_offset: float = Field(description="DOWN offset AFTER inventory skew")
    raw_size_up: float = Field(description="UP size from skew formula")
    raw_size_down: float = Field(description="DOWN size from skew formula")

    # Layer 4: Edge Check
    edge_up: float = Field(description="Edge vs market: ask - bid")
    edge_down: float = Field(description="Edge vs market for DOWN")
    skip_reason_up: str | None = Field(default=None, description="Why UP was skipped")
    skip_reason_down: str | None = Field(default=None, description="Why DOWN was skipped")

    @computed_field  # type: ignore[prop-decorator]
    @property
    def combined_bid(self) -> float | None:
        """Combined bid if quoting both sides."""
        if self.bid_up is not None and self.bid_down is not None:
            return self.bid_up + self.bid_down
        return None

    @computed_field  # type: ignore[prop-decorator]
    @property
    def profit_per_pair(self) -> float | None:
        """Profit per pair if both sides fill."""
        if self.combined_bid is not None:
            return 1.0 - self.combined_bid
        return None
