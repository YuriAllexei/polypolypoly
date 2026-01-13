"""Performance metrics for backtesting and optimization."""

import math
from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray


@dataclass
class MetricsSummary:
    """Summary of backtest performance metrics."""

    total_pnl: float
    """Total profit/loss in dollars."""

    realized_pnl: float
    """PnL from redeemed pairs."""

    unrealized_pnl: float
    """PnL from unmatched inventory (at current prices)."""

    total_fills: int
    """Total number of fills (up + down)."""

    up_fills: int
    """Number of UP fills."""

    down_fills: int
    """Number of DOWN fills."""

    fill_rate: float
    """Percentage of quotes that resulted in fills."""

    avg_spread_captured: float
    """Average spread captured per fill."""

    sharpe_ratio: float | None
    """Sharpe ratio of returns (None if insufficient data)."""

    max_drawdown: float
    """Maximum peak-to-trough drawdown."""

    final_imbalance: float
    """Final inventory imbalance q."""

    final_pairs: float
    """Final number of redeemable pairs."""

    avg_combined_cost: float
    """Average combined cost per pair."""


def calculate_sharpe_ratio(
    returns: NDArray[np.float64], risk_free_rate: float = 0.0
) -> float | None:
    """Calculate annualized Sharpe ratio.

    Args:
        returns: Array of period returns
        risk_free_rate: Risk-free rate (default 0)

    Returns:
        Sharpe ratio or None if insufficient data
    """
    if len(returns) < 2:
        return None

    excess_returns = returns - risk_free_rate
    mean_return = np.mean(excess_returns)
    std_return = np.std(excess_returns, ddof=1)

    if std_return == 0:
        return None

    # Annualize assuming 15-minute periods, ~35000 periods/year
    periods_per_year = 35040  # 365 * 24 * 4
    return float(mean_return / std_return * math.sqrt(periods_per_year))


def calculate_max_drawdown(equity_curve: NDArray[np.float64]) -> float:
    """Calculate maximum drawdown from equity curve.

    Args:
        equity_curve: Array of cumulative PnL values

    Returns:
        Maximum drawdown as a positive value
    """
    if len(equity_curve) < 2:
        return 0.0

    # Calculate running maximum
    running_max = np.maximum.accumulate(equity_curve)

    # Calculate drawdown at each point
    drawdowns = running_max - equity_curve

    return float(np.max(drawdowns))


def calculate_fill_rate(
    total_quotes: int, total_fills: int
) -> float:
    """Calculate fill rate percentage.

    Args:
        total_quotes: Total quotes placed
        total_fills: Total fills received

    Returns:
        Fill rate as percentage (0-100)
    """
    if total_quotes == 0:
        return 0.0
    return 100.0 * total_fills / total_quotes


def calculate_metrics(
    pnl_history: list[float],
    fills: list[dict[str, float]],
    final_inventory_up: float,
    final_inventory_down: float,
    final_up_avg: float,
    final_down_avg: float,
    total_quotes: int,
    final_market_up_mid: float = 0.5,
    final_market_down_mid: float = 0.5,
) -> MetricsSummary:
    """Calculate comprehensive metrics from backtest results.

    Args:
        pnl_history: List of PnL at each timestep
        fills: List of fill records with keys: side, qty, price, timestamp
        final_inventory_up: Final UP quantity held
        final_inventory_down: Final DOWN quantity held
        final_up_avg: Final average UP cost
        final_down_avg: Final average DOWN cost
        total_quotes: Total number of quotes placed
        final_market_up_mid: Final UP mid price for unrealized PnL
        final_market_down_mid: Final DOWN mid price for unrealized PnL

    Returns:
        MetricsSummary with all calculated metrics
    """
    # Convert to numpy for calculations
    pnl_array = np.array(pnl_history, dtype=np.float64)

    # Calculate returns (differences in PnL)
    if len(pnl_array) > 1:
        returns = np.diff(pnl_array)
    else:
        returns = np.array([], dtype=np.float64)

    # Count fills by side
    up_fills = sum(1 for f in fills if f.get("side") == "up")
    down_fills = sum(1 for f in fills if f.get("side") == "down")
    total_fills = up_fills + down_fills

    # Calculate spreads captured
    spreads = []
    for f in fills:
        # Spread captured = how much below mid we bought
        # This is a simplification - actual spread depends on execution
        spreads.append(f.get("spread_captured", 0.0))
    avg_spread = float(np.mean(spreads)) if spreads else 0.0

    # Inventory metrics
    pairs = min(final_inventory_up, final_inventory_down)
    total_inv = final_inventory_up + final_inventory_down
    imbalance = (
        (final_inventory_up - final_inventory_down) / total_inv
        if total_inv > 0
        else 0.0
    )

    # PnL breakdown
    combined_avg = final_up_avg + final_down_avg
    realized_pnl = pairs * (1.0 - combined_avg)

    # Unrealized PnL from unmatched inventory
    unmatched_up = final_inventory_up - pairs
    unmatched_down = final_inventory_down - pairs
    unrealized_pnl = (
        unmatched_up * (final_market_up_mid - final_up_avg)
        + unmatched_down * (final_market_down_mid - final_down_avg)
    )

    total_pnl = realized_pnl + unrealized_pnl

    return MetricsSummary(
        total_pnl=total_pnl,
        realized_pnl=realized_pnl,
        unrealized_pnl=unrealized_pnl,
        total_fills=total_fills,
        up_fills=up_fills,
        down_fills=down_fills,
        fill_rate=calculate_fill_rate(total_quotes, total_fills),
        avg_spread_captured=avg_spread,
        sharpe_ratio=calculate_sharpe_ratio(returns),
        max_drawdown=calculate_max_drawdown(pnl_array),
        final_imbalance=imbalance,
        final_pairs=pairs,
        avg_combined_cost=combined_avg,
    )
