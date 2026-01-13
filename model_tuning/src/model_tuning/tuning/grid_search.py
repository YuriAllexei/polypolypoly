"""Grid search for exhaustive parameter exploration."""

from dataclasses import dataclass, field
from itertools import product
from typing import Any

import pandas as pd
from rich.progress import Progress, SpinnerColumn, TextColumn, BarColumn, TaskProgressColumn

from model_tuning.core.quoter import InventoryMMQuoter, QuoterParams
from model_tuning.tuning.backtester import Backtester, BacktestResult, MarketTick


# Type alias for parameter grid
ParameterGrid = dict[str, list[float]]


@dataclass
class GridSearchResult:
    """Results from a grid search run."""

    results: list[BacktestResult] = field(default_factory=list)
    """All backtest results."""

    param_combinations: int = 0
    """Total number of parameter combinations tested."""

    def to_dataframe(self) -> pd.DataFrame:
        """Convert results to a pandas DataFrame for analysis.

        Returns:
            DataFrame with parameters and metrics for each run.
        """
        rows = []
        for result in self.results:
            if result.params is None:
                continue

            row: dict[str, Any] = {}

            # Add parameters
            for key, value in result.params.model_dump().items():
                row[key] = value

            # Add metrics
            metrics = result.metrics
            row["total_pnl"] = metrics.total_pnl
            row["realized_pnl"] = metrics.realized_pnl
            row["unrealized_pnl"] = metrics.unrealized_pnl
            row["total_fills"] = metrics.total_fills
            row["up_fills"] = metrics.up_fills
            row["down_fills"] = metrics.down_fills
            row["fill_rate"] = metrics.fill_rate
            row["avg_spread_captured"] = metrics.avg_spread_captured
            row["sharpe_ratio"] = metrics.sharpe_ratio
            row["max_drawdown"] = metrics.max_drawdown
            row["final_imbalance"] = metrics.final_imbalance
            row["final_pairs"] = metrics.final_pairs

            rows.append(row)

        return pd.DataFrame(rows)

    def best_by_metric(
        self,
        metric: str = "total_pnl",
        ascending: bool = False,
    ) -> BacktestResult | None:
        """Get the best result by a specific metric.

        Args:
            metric: Metric name to sort by (e.g., 'total_pnl', 'sharpe_ratio')
            ascending: If True, lower is better; if False, higher is better

        Returns:
            Best BacktestResult or None if no results
        """
        if not self.results:
            return None

        df = self.to_dataframe()
        if metric not in df.columns:
            raise ValueError(f"Unknown metric: {metric}")

        # Handle None values in sharpe_ratio
        if metric == "sharpe_ratio":
            df = df.dropna(subset=["sharpe_ratio"])
            if df.empty:
                return None

        df_sorted = df.sort_values(metric, ascending=ascending)
        best_idx = df_sorted.index[0]

        return self.results[best_idx]

    def top_n(
        self,
        n: int = 10,
        metric: str = "total_pnl",
        ascending: bool = False,
    ) -> pd.DataFrame:
        """Get top N results sorted by metric.

        Args:
            n: Number of results to return
            metric: Metric to sort by
            ascending: Sort order

        Returns:
            DataFrame with top N results
        """
        df = self.to_dataframe()
        if metric not in df.columns:
            raise ValueError(f"Unknown metric: {metric}")

        return df.sort_values(metric, ascending=ascending).head(n)

    def summary_stats(self) -> dict[str, dict[str, float]]:
        """Calculate summary statistics for key metrics.

        Returns:
            Dict mapping metric name to dict of stats (mean, std, min, max)
        """
        df = self.to_dataframe()
        metrics_to_summarize = [
            "total_pnl",
            "fill_rate",
            "sharpe_ratio",
            "max_drawdown",
            "final_imbalance",
        ]

        stats: dict[str, dict[str, float]] = {}
        for metric in metrics_to_summarize:
            if metric not in df.columns:
                continue

            values = df[metric].dropna()
            if len(values) == 0:
                continue

            stats[metric] = {
                "mean": float(values.mean()),
                "std": float(values.std()),
                "min": float(values.min()),
                "max": float(values.max()),
            }

        return stats


class GridSearcher:
    """Exhaustive grid search over quoter parameters.

    Runs backtests for all combinations of specified parameter values.
    """

    def __init__(
        self,
        backtester: Backtester,
        ticks: list[MarketTick],
    ) -> None:
        """Initialize grid searcher.

        Args:
            backtester: Backtester instance for evaluation
            ticks: Market data for backtesting
        """
        self.backtester = backtester
        self.ticks = ticks

    def search(
        self,
        grid: ParameterGrid,
        fixed_params: dict[str, float] | None = None,
        show_progress: bool = True,
    ) -> GridSearchResult:
        """Run exhaustive grid search.

        Args:
            grid: Dict mapping parameter names to lists of values to try
            fixed_params: Optional dict of parameters to hold constant
            show_progress: Whether to show progress bar

        Returns:
            GridSearchResult with all backtest results
        """
        fixed = fixed_params or {}

        # Generate all parameter combinations
        param_names = list(grid.keys())
        param_values = list(grid.values())
        combinations = list(product(*param_values))

        total = len(combinations)
        results: list[BacktestResult] = []

        if show_progress:
            with Progress(
                SpinnerColumn(),
                TextColumn("[progress.description]{task.description}"),
                BarColumn(),
                TaskProgressColumn(),
                TextColumn("[cyan]{task.completed}/{task.total}"),
            ) as progress:
                task = progress.add_task("Running grid search...", total=total)

                for combo in combinations:
                    # Build params dict from combination
                    params_dict = dict(zip(param_names, combo, strict=True))
                    params_dict.update(fixed)

                    result = self._run_single(params_dict)
                    results.append(result)

                    progress.advance(task)
        else:
            for combo in combinations:
                params_dict = dict(zip(param_names, combo, strict=True))
                params_dict.update(fixed)

                result = self._run_single(params_dict)
                results.append(result)

        return GridSearchResult(
            results=results,
            param_combinations=total,
        )

    def _run_single(self, params_dict: dict[str, float]) -> BacktestResult:
        """Run a single backtest with given parameters.

        Args:
            params_dict: Parameter values

        Returns:
            BacktestResult from the backtest
        """
        params = QuoterParams(**params_dict)
        quoter = InventoryMMQuoter(params)
        return self.backtester.run(quoter, self.ticks)
