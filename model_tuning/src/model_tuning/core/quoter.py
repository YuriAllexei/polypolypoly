"""4-Layer Inventory Market Maker Quoter for Polymarket binary markets.

The 4-Layer Framework:
1. Oracle-Adjusted Offset: React to oracle direction
2. Adverse Selection: Widen spread near resolution (Glosten-Milgrom)
3. Inventory Skew: Balance positions (affects BOTH prices and sizes)
4. Edge Check: Don't overpay

Theoretical Foundation:
- O'Hara, M. (1995). *Market Microstructure Theory*
  - Chapter 2: Inventory Models
  - Chapter 3: Information-Based Models (Glosten-Milgrom)
"""

import math

from pydantic import BaseModel, Field

from model_tuning.core.models import Inventory, Market, Oracle, QuoteResult
from model_tuning.core.utils import snap_to_tick


class QuoterParams(BaseModel):
    """Parameters for the InventoryMMQuoter.

    Organized by the 4-layer framework.
    """

    # Layer 1: Oracle-Adjusted Offset
    oracle_sensitivity: float = Field(
        default=5.0,
        description="How much oracle shifts offset. oracle_adj = distance_pct x sensitivity",
    )

    # Layer 2: Adverse Selection (Glosten-Milgrom)
    base_spread: float = Field(
        default=0.02,
        description="Base offset from best_ask (2c) before oracle adjustment",
    )
    p_informed_base: float = Field(
        default=0.2,
        description="Base probability of informed trader (20%)",
    )
    time_decay_minutes: float = Field(
        default=5.0,
        description="Time constant: p_informed = base x exp(-minutes / decay)",
    )

    # Layer 3: Inventory Skew
    gamma_inv: float = Field(
        default=0.5,
        description="Offset multiplier sensitivity: mult = 1 + gamma x imbalance",
    )
    lambda_size: float = Field(
        default=1.0,
        description="Size sensitivity: size = base x exp(-lambda x imbalance)",
    )
    base_size: float = Field(
        default=50.0,
        description="Base order size when balanced",
    )

    # Layer 4: Edge Check
    edge_threshold: float = Field(
        default=0.01,
        description="Minimum edge (1c) to quote",
    )
    min_offset: float = Field(
        default=0.01,
        description="Minimum offset from best_ask (1c)",
    )


class InventoryMMQuoter:
    """Inventory Market Maker for Polymarket 15-minute binary markets.

    4-Layer Framework:
    1. Oracle-Adjusted Offset: React to oracle direction
    2. Adverse Selection: Widen spread near resolution
    3. Inventory Skew: Balance positions (affects BOTH prices and sizes)
    4. Edge Check: Don't overpay
    """

    def __init__(self, params: QuoterParams | None = None) -> None:
        """Initialize quoter with parameters.

        Args:
            params: Quoter parameters. Uses defaults if not provided.
        """
        self.params = params or QuoterParams()

    @classmethod
    def from_dict(cls, config: dict[str, float]) -> "InventoryMMQuoter":
        """Create quoter from a dictionary of parameters."""
        return cls(QuoterParams(**config))

    def calculate_oracle_adjusted_offsets(
        self, oracle: Oracle, base_offset: float
    ) -> tuple[float, float, float]:
        """Layer 1: Adjust offset from best_ask based on oracle direction.

        When oracle favors UP (above threshold):
          - UP offset DECREASES (tighter bid, more aggressive)
          - DOWN offset INCREASES (wider bid, protection from dumps)

        Returns:
            (oracle_adj, up_offset, down_offset)
        """
        oracle_adj = oracle.distance_pct * self.params.oracle_sensitivity
        up_offset = max(self.params.min_offset, base_offset - oracle_adj)
        down_offset = max(self.params.min_offset, base_offset + oracle_adj)
        return oracle_adj, up_offset, down_offset

    def calculate_adverse_selection(
        self, minutes_to_resolution: float
    ) -> tuple[float, float]:
        """Layer 2: Widen spread near resolution when informed traders dominate.

        Formula: p_informed = base x exp(-minutes / decay)
                 spread = base_spread x (1 + 3 x p_informed)

        Returns:
            (p_informed, spread)
        """
        p_informed = self.params.p_informed_base * math.exp(
            -minutes_to_resolution / self.params.time_decay_minutes
        )
        p_informed = min(0.8, p_informed)  # Cap at 80%
        spread = self.params.base_spread * (1 + 3 * p_informed)
        return p_informed, spread

    def calculate_inventory_skew(
        self, inventory: Inventory
    ) -> tuple[float, float, float, float]:
        """Layer 3: Adjust offsets and sizes based on inventory imbalance.

        Offset multiplier: mult = 1 + gamma x imbalance
        Size: size = base x exp(-lambda x imbalance)

        Returns:
            (spread_mult_up, spread_mult_down, size_up, size_down)
        """
        q = inventory.imbalance

        # SPREAD MULTIPLIER (affects final offset)
        spread_mult_up = 1 + self.params.gamma_inv * q  # >1 when overweight UP
        spread_mult_down = 1 - self.params.gamma_inv * q  # <1 when overweight UP

        # SIZE (affects order quantity)
        size_up = self.params.base_size * math.exp(
            -self.params.lambda_size * q
        )  # Smaller when overweight UP
        size_down = self.params.base_size * math.exp(
            self.params.lambda_size * q
        )  # Bigger when overweight UP

        return spread_mult_up, spread_mult_down, size_up, size_down

    def check_edge(
        self, bid: float, market_ask: float
    ) -> tuple[bool, float, str | None]:
        """Layer 4: Check if we have sufficient edge vs market.

        Returns:
            (should_quote, edge, skip_reason)
        """
        edge = market_ask - bid
        if edge < self.params.edge_threshold:
            return (
                False,
                edge,
                f"edge {edge:.3f} < threshold {self.params.edge_threshold}",
            )
        return True, edge, None

    def quote(
        self,
        inventory: Inventory,
        market: Market,
        oracle: Oracle,
        minutes_to_resolution: float,
    ) -> QuoteResult:
        """Generate quotes using the 4-layer framework.

        Args:
            inventory: Current position in UP and DOWN tokens
            market: Current orderbook state
            oracle: External price feed
            minutes_to_resolution: Time left until market resolves

        Returns:
            QuoteResult with all intermediate calculations for debugging
        """
        # Layer 2: Adverse selection (base spread)
        p_informed, base_spread = self.calculate_adverse_selection(
            minutes_to_resolution
        )

        # Layer 1: Oracle-adjusted offsets
        oracle_adj, raw_up_offset, raw_down_offset = (
            self.calculate_oracle_adjusted_offsets(oracle, base_spread)
        )

        # Layer 3: Inventory skew
        spread_mult_up, spread_mult_down, raw_size_up, raw_size_down = (
            self.calculate_inventory_skew(inventory)
        )

        # Apply inventory skew to offsets
        final_up_offset = raw_up_offset * spread_mult_up
        final_down_offset = raw_down_offset * spread_mult_down

        # Calculate bids (from best_bid, not best_ask as in notebook)
        # This matches the notebook logic more closely
        raw_bid_up = market.best_bid_up - final_up_offset
        raw_bid_down = market.best_bid_down - final_down_offset

        # Snap to tick
        bid_up = snap_to_tick(raw_bid_up)
        bid_down = snap_to_tick(raw_bid_down)

        # Layer 4: Edge check
        quote_up, edge_up, skip_up = self.check_edge(bid_up, market.best_ask_up)
        quote_down, edge_down, skip_down = self.check_edge(
            bid_down, market.best_ask_down
        )

        return QuoteResult(
            # Final quotes
            bid_up=bid_up if quote_up else None,
            size_up=round(raw_size_up) if quote_up else 0,
            bid_down=bid_down if quote_down else None,
            size_down=round(raw_size_down) if quote_down else 0,
            # Layer 1
            oracle_adj=oracle_adj,
            raw_up_offset=raw_up_offset,
            raw_down_offset=raw_down_offset,
            # Layer 2
            p_informed=p_informed,
            base_spread=base_spread,
            # Layer 3
            inventory_q=inventory.imbalance,
            spread_mult_up=spread_mult_up,
            spread_mult_down=spread_mult_down,
            final_up_offset=final_up_offset,
            final_down_offset=final_down_offset,
            raw_size_up=raw_size_up,
            raw_size_down=raw_size_down,
            # Layer 4
            edge_up=edge_up,
            edge_down=edge_down,
            skip_reason_up=skip_up,
            skip_reason_down=skip_down,
        )
