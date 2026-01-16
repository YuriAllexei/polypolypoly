"""Quoter protocols and implementations for fill-driven simulation.

Provides a simple Protocol for quoters that generate bids based on
orderbook state, plus a BrainDeadQuoter implementation for baseline testing.
"""

from dataclasses import dataclass
from typing import Protocol

from model_tuning.core.utils import snap_to_tick
from model_tuning.simulation.models import (
    OrderbookSnapshot,
    OracleSnapshot,
)


@dataclass
class SimpleQuote:
    """Simple quote with bid prices and sizes for both sides.

    Represents a market maker's willingness to buy UP and DOWN tokens.
    """

    bid_up: float | None
    """Bid price for UP token (None = not quoting)"""

    size_up: float
    """Size to bid for UP token"""

    bid_down: float | None
    """Bid price for DOWN token (None = not quoting)"""

    size_down: float
    """Size to bid for DOWN token"""


class SimulationQuoter(Protocol):
    """Protocol for quoters used in fill-driven simulation.

    Any quoter implementing this protocol can be used with FillDrivenSimulator.
    """

    def quote(
        self,
        orderbook: OrderbookSnapshot,
        oracle: OracleSnapshot | None = None,
    ) -> SimpleQuote:
        """Generate a quote based on current orderbook and optional oracle.

        Args:
            orderbook: Current orderbook state
            oracle: Optional oracle price data

        Returns:
            SimpleQuote with bid prices and sizes
        """
        ...


class BrainDeadQuoter:
    """Simple baseline quoter: best_bid - offset, fixed size on both sides.

    This is the simplest possible quoter for baseline testing:
    - Quotes at best_bid - offset (default 2 ticks = 0.02)
    - Fixed size on both sides (default 50)
    - Ignores oracle data entirely

    Use this to establish a baseline before testing more sophisticated quoters.
    """

    def __init__(self, offset: float = 0.02, size: float = 50.0) -> None:
        """Initialize BrainDeadQuoter.

        Args:
            offset: How far below best_bid to quote (default 0.02 = 2 ticks)
            size: Size to quote on each side (default 50)
        """
        self.offset = offset
        self.size = size

    def quote(
        self,
        orderbook: OrderbookSnapshot,
        oracle: OracleSnapshot | None = None,
    ) -> SimpleQuote:
        """Generate quote at best_bid - offset on both sides.

        Args:
            orderbook: Current orderbook state
            oracle: Ignored (BrainDeadQuoter doesn't use oracle)

        Returns:
            SimpleQuote with bids at best_bid - offset, fixed size
        """
        # Get best bids, defaulting to 0.5 if no bids exist
        up_best_bid = orderbook.up.best_bid
        down_best_bid = orderbook.down.best_bid

        # Calculate bid prices (None if no best_bid exists)
        bid_up = None
        if up_best_bid is not None:
            bid_up = snap_to_tick(up_best_bid - self.offset)
            # Ensure bid is positive and valid
            if bid_up <= 0:
                bid_up = None

        bid_down = None
        if down_best_bid is not None:
            bid_down = snap_to_tick(down_best_bid - self.offset)
            if bid_down <= 0:
                bid_down = None

        return SimpleQuote(
            bid_up=bid_up,
            size_up=self.size,
            bid_down=bid_down,
            size_down=self.size,
        )
