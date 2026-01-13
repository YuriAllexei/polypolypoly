"""Optuna-based parameter optimizer for the quoter."""

from enum import Enum
from typing import Any

import optuna
from optuna.trial import Trial

from model_tuning.core.quoter import InventoryMMQuoter, QuoterParams
from model_tuning.tuning.backtester import Backtester, BacktestResult, MarketTick


class ObjectiveType(str, Enum):
    """Optimization objective types."""

    TOTAL_PNL = "total_pnl"
    """Maximize total PnL."""

    SHARPE = "sharpe"
    """Maximize Sharpe ratio."""

    RISK_ADJUSTED = "risk_adjusted"
    """Maximize PnL / max_drawdown."""

    BALANCED = "balanced"
    """Weighted combination of PnL, Sharpe, and fill rate."""


class QuoterOptimizer:
    """Optuna-based hyperparameter optimizer for InventoryMMQuoter.

    Uses backtesting to evaluate parameter configurations.
    """

    def __init__(
        self,
        backtester: Backtester,
        ticks: list[MarketTick],
        objective: ObjectiveType = ObjectiveType.TOTAL_PNL,
        n_trials: int = 100,
        random_seed: int | None = 42,
    ) -> None:
        """Initialize optimizer.

        Args:
            backtester: Backtester instance for evaluation
            ticks: Market data for backtesting
            objective: Optimization objective
            n_trials: Number of optimization trials
            random_seed: Random seed for reproducibility
        """
        self.backtester = backtester
        self.ticks = ticks
        self.objective = objective
        self.n_trials = n_trials
        self.random_seed = random_seed
        self.study: optuna.Study | None = None
        self.best_result: BacktestResult | None = None

    def _suggest_params(self, trial: Trial) -> QuoterParams:
        """Suggest parameter values for a trial.

        Args:
            trial: Optuna trial

        Returns:
            QuoterParams with suggested values
        """
        return QuoterParams(
            # Layer 1: Oracle
            oracle_sensitivity=trial.suggest_float(
                "oracle_sensitivity", 1.0, 20.0, log=True
            ),
            # Layer 2: Adverse Selection
            base_spread=trial.suggest_float("base_spread", 0.005, 0.05),
            p_informed_base=trial.suggest_float("p_informed_base", 0.05, 0.5),
            time_decay_minutes=trial.suggest_float(
                "time_decay_minutes", 1.0, 15.0
            ),
            # Layer 3: Inventory Skew
            gamma_inv=trial.suggest_float("gamma_inv", 0.1, 2.0),
            lambda_size=trial.suggest_float("lambda_size", 0.5, 3.0),
            base_size=trial.suggest_float("base_size", 10.0, 200.0),
            # Layer 4: Edge Check
            edge_threshold=trial.suggest_float("edge_threshold", 0.001, 0.03),
            min_offset=trial.suggest_float("min_offset", 0.005, 0.03),
        )

    def _calculate_objective(self, result: BacktestResult) -> float:
        """Calculate objective value from backtest result.

        Args:
            result: Backtest result

        Returns:
            Objective value (higher is better for maximization)
        """
        metrics = result.metrics

        if self.objective == ObjectiveType.TOTAL_PNL:
            return metrics.total_pnl

        elif self.objective == ObjectiveType.SHARPE:
            if metrics.sharpe_ratio is None:
                return -1000.0  # Penalize if no Sharpe
            return metrics.sharpe_ratio

        elif self.objective == ObjectiveType.RISK_ADJUSTED:
            if metrics.max_drawdown == 0:
                return metrics.total_pnl * 100  # No drawdown is great
            return metrics.total_pnl / max(0.01, metrics.max_drawdown)

        elif self.objective == ObjectiveType.BALANCED:
            # Weighted combination
            pnl_score = metrics.total_pnl
            sharpe_score = (metrics.sharpe_ratio or 0) * 10
            fill_score = metrics.fill_rate / 10
            balance_score = (1 - abs(metrics.final_imbalance)) * 20

            return pnl_score + sharpe_score + fill_score + balance_score

        return metrics.total_pnl

    def _objective_fn(self, trial: Trial) -> float:
        """Optuna objective function.

        Args:
            trial: Optuna trial

        Returns:
            Objective value
        """
        params = self._suggest_params(trial)
        quoter = InventoryMMQuoter(params)
        result = self.backtester.run(quoter, self.ticks)

        # Store best result
        obj_value = self._calculate_objective(result)
        if (
            self.best_result is None
            or obj_value > self._calculate_objective(self.best_result)
        ):
            self.best_result = result

        # Log intermediate values for analysis
        trial.set_user_attr("total_pnl", result.metrics.total_pnl)
        trial.set_user_attr("sharpe_ratio", result.metrics.sharpe_ratio)
        trial.set_user_attr("fill_rate", result.metrics.fill_rate)
        trial.set_user_attr("max_drawdown", result.metrics.max_drawdown)
        trial.set_user_attr("final_imbalance", result.metrics.final_imbalance)

        return obj_value

    def optimize(
        self,
        show_progress: bool = True,
        callbacks: list[Any] | None = None,
    ) -> QuoterParams:
        """Run optimization.

        Args:
            show_progress: Whether to show progress bar
            callbacks: Optional Optuna callbacks

        Returns:
            Best parameters found
        """
        sampler = optuna.samplers.TPESampler(seed=self.random_seed)
        self.study = optuna.create_study(
            direction="maximize",
            sampler=sampler,
        )

        # Suppress Optuna logging if not showing progress
        if not show_progress:
            optuna.logging.set_verbosity(optuna.logging.WARNING)

        self.study.optimize(
            self._objective_fn,
            n_trials=self.n_trials,
            show_progress_bar=show_progress,
            callbacks=callbacks,
        )

        return QuoterParams(**self.study.best_params)

    def get_param_importance(self) -> dict[str, float]:
        """Get parameter importance scores.

        Returns:
            Dict mapping param name to importance score
        """
        if self.study is None:
            raise RuntimeError("Must call optimize() first")

        return optuna.importance.get_param_importances(self.study)

    def get_optimization_history(self) -> list[dict[str, Any]]:
        """Get history of all trials.

        Returns:
            List of trial info dicts
        """
        if self.study is None:
            raise RuntimeError("Must call optimize() first")

        history = []
        for trial in self.study.trials:
            history.append(
                {
                    "number": trial.number,
                    "value": trial.value,
                    "params": trial.params,
                    "user_attrs": trial.user_attrs,
                    "state": trial.state.name,
                }
            )
        return history
