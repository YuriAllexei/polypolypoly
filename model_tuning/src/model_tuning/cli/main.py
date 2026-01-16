"""CLI commands for model tuning."""

from pathlib import Path
from typing import Annotated, Optional

import typer
import yaml
from rich import print as rprint
from rich.console import Console
from rich.table import Table

from model_tuning.core.quoter import InventoryMMQuoter, QuoterParams
from model_tuning.data.loaders import generate_synthetic_ticks, load_ticks_from_csv
from model_tuning.tuning.backtester import Backtester, FillSimulator
from model_tuning.tuning.grid_search import GridSearcher
from model_tuning.tuning.optimizer import ObjectiveType, QuoterOptimizer

app = typer.Typer(
    name="model-tuning",
    help="Market-making quoter model tuning for Polymarket binary markets",
    add_completion=False,
)
console = Console()


@app.command()
def backtest(
    config: Annotated[
        Optional[Path],
        typer.Option("--config", "-c", help="Path to YAML config file"),
    ] = None,
    data: Annotated[
        Optional[Path],
        typer.Option("--data", "-d", help="Path to market data CSV"),
    ] = None,
    duration: Annotated[
        float,
        typer.Option("--duration", help="Duration in minutes for synthetic data"),
    ] = 15.0,
    seed: Annotated[
        int,
        typer.Option("--seed", "-s", help="Random seed"),
    ] = 42,
    verbose: Annotated[
        bool,
        typer.Option("--verbose", "-v", help="Verbose output"),
    ] = False,
) -> None:
    """Run a single backtest with given parameters.

    If no data file is provided, generates synthetic data.
    """
    # Load or generate data
    if data:
        rprint(f"[blue]Loading data from {data}...[/blue]")
        ticks = load_ticks_from_csv(data)
    else:
        rprint(f"[blue]Generating {duration:.1f} minutes of synthetic data...[/blue]")
        ticks = generate_synthetic_ticks(
            duration_minutes=duration,
            random_seed=seed,
        )

    rprint(f"[green]Loaded {len(ticks)} ticks[/green]")

    # Load or use default params
    if config:
        with open(config) as f:
            config_dict = yaml.safe_load(f)
        params = QuoterParams(**config_dict.get("quoter", {}))
    else:
        params = QuoterParams()

    if verbose:
        rprint("\n[bold]Quoter Parameters:[/bold]")
        for key, value in params.model_dump().items():
            rprint(f"  {key}: {value}")

    # Run backtest
    quoter = InventoryMMQuoter(params)
    backtester = Backtester(
        fill_simulator=FillSimulator(random_seed=seed),
    )

    rprint("\n[blue]Running backtest...[/blue]")
    result = backtester.run(quoter, ticks)

    # Display results
    _display_metrics(result.metrics)

    if verbose and result.fills:
        rprint(f"\n[bold]Sample fills ({min(5, len(result.fills))} of {len(result.fills)}):[/bold]")
        for fill in result.fills[:5]:
            rprint(
                f"  {fill.side.upper()} {fill.qty:.1f} @ {fill.price:.2f} "
                f"(spread: {fill.spread_captured*100:.1f}c)"
            )


@app.command()
def tune(
    data: Annotated[
        Optional[Path],
        typer.Option("--data", "-d", help="Path to market data CSV"),
    ] = None,
    objective: Annotated[
        ObjectiveType,
        typer.Option("--objective", "-o", help="Optimization objective"),
    ] = ObjectiveType.TOTAL_PNL,
    trials: Annotated[
        int,
        typer.Option("--trials", "-n", help="Number of optimization trials"),
    ] = 100,
    duration: Annotated[
        float,
        typer.Option("--duration", help="Duration in minutes for synthetic data"),
    ] = 15.0,
    seed: Annotated[
        int,
        typer.Option("--seed", "-s", help="Random seed"),
    ] = 42,
    output: Annotated[
        Optional[Path],
        typer.Option("--output", help="Output YAML file for best params"),
    ] = None,
) -> None:
    """Run parameter optimization using Optuna.

    Finds optimal quoter parameters by backtesting many configurations.
    """
    # Load or generate data
    if data:
        rprint(f"[blue]Loading data from {data}...[/blue]")
        ticks = load_ticks_from_csv(data)
    else:
        rprint(f"[blue]Generating {duration:.1f} minutes of synthetic data...[/blue]")
        ticks = generate_synthetic_ticks(
            duration_minutes=duration,
            random_seed=seed,
        )

    rprint(f"[green]Loaded {len(ticks)} ticks[/green]")
    rprint(f"[blue]Objective: {objective.value}[/blue]")
    rprint(f"[blue]Running {trials} trials...[/blue]\n")

    # Set up optimizer
    backtester = Backtester(
        fill_simulator=FillSimulator(random_seed=seed),
    )
    optimizer = QuoterOptimizer(
        backtester=backtester,
        ticks=ticks,
        objective=objective,
        n_trials=trials,
        random_seed=seed,
    )

    # Run optimization
    best_params = optimizer.optimize(show_progress=True)

    # Display results
    rprint("\n[bold green]Optimization Complete![/bold green]\n")

    rprint("[bold]Best Parameters:[/bold]")
    params_table = Table(show_header=True, header_style="bold")
    params_table.add_column("Parameter")
    params_table.add_column("Value", justify="right")

    for key, value in best_params.model_dump().items():
        params_table.add_row(key, f"{value:.4f}")
    console.print(params_table)

    # Show best result metrics
    if optimizer.best_result:
        rprint("\n[bold]Best Result Metrics:[/bold]")
        _display_metrics(optimizer.best_result.metrics)

    # Show parameter importance
    try:
        importance = optimizer.get_param_importance()
        rprint("\n[bold]Parameter Importance:[/bold]")
        imp_table = Table(show_header=True, header_style="bold")
        imp_table.add_column("Parameter")
        imp_table.add_column("Importance", justify="right")

        for param, imp in sorted(importance.items(), key=lambda x: -x[1]):
            imp_table.add_row(param, f"{imp:.3f}")
        console.print(imp_table)
    except Exception:
        pass  # Importance calculation may fail with few trials

    # Save to file if requested
    if output:
        config_dict = {"quoter": best_params.model_dump()}
        with open(output, "w") as f:
            yaml.dump(config_dict, f, default_flow_style=False)
        rprint(f"\n[green]Saved best params to {output}[/green]")


@app.command()
def analyze(
    config: Annotated[
        Path,
        typer.Argument(help="Path to YAML config file to analyze"),
    ],
) -> None:
    """Analyze a quoter configuration.

    Displays the parameters and runs a quick backtest.
    """
    with open(config) as f:
        config_dict = yaml.safe_load(f)

    params = QuoterParams(**config_dict.get("quoter", {}))

    rprint("[bold]Configuration Analysis[/bold]\n")

    # Display params
    params_table = Table(show_header=True, header_style="bold", title="Parameters")
    params_table.add_column("Parameter")
    params_table.add_column("Value", justify="right")
    params_table.add_column("Description")

    param_descriptions = {
        "oracle_sensitivity": "Oracle direction impact",
        "base_spread": "Base offset from best ask",
        "p_informed_base": "Base informed trader probability",
        "time_decay_minutes": "Adverse selection decay",
        "gamma_inv": "Inventory skew on spread",
        "lambda_size": "Inventory skew on size",
        "base_size": "Base order size",
        "edge_threshold": "Minimum edge to quote",
        "min_offset": "Minimum offset from ask",
    }

    for key, value in params.model_dump().items():
        desc = param_descriptions.get(key, "")
        params_table.add_row(key, f"{value:.4f}", desc)
    console.print(params_table)

    # Quick backtest
    rprint("\n[blue]Running quick backtest (5 min synthetic)...[/blue]")
    ticks = generate_synthetic_ticks(duration_minutes=5.0, random_seed=42)
    quoter = InventoryMMQuoter(params)
    backtester = Backtester(fill_simulator=FillSimulator(random_seed=42))
    result = backtester.run(quoter, ticks)

    _display_metrics(result.metrics)


def _display_metrics(metrics: "model_tuning.tuning.metrics.MetricsSummary") -> None:  # type: ignore[name-defined]
    """Display metrics in a formatted table."""
    table = Table(show_header=True, header_style="bold", title="Performance Metrics")
    table.add_column("Metric")
    table.add_column("Value", justify="right")

    # PnL metrics
    pnl_color = "green" if metrics.total_pnl > 0 else "red"
    table.add_row("Total PnL", f"[{pnl_color}]${metrics.total_pnl:.2f}[/{pnl_color}]")
    table.add_row("Realized PnL", f"${metrics.realized_pnl:.2f}")
    table.add_row("Unrealized PnL", f"${metrics.unrealized_pnl:.2f}")

    # Fill metrics
    table.add_row("Total Fills", f"{metrics.total_fills}")
    table.add_row("UP Fills", f"{metrics.up_fills}")
    table.add_row("DOWN Fills", f"{metrics.down_fills}")
    table.add_row("Fill Rate", f"{metrics.fill_rate:.1f}%")
    table.add_row("Avg Spread Captured", f"{metrics.avg_spread_captured*100:.1f}c")

    # Risk metrics
    sharpe_str = f"{metrics.sharpe_ratio:.2f}" if metrics.sharpe_ratio else "N/A"
    table.add_row("Sharpe Ratio", sharpe_str)
    table.add_row("Max Drawdown", f"${metrics.max_drawdown:.2f}")

    # Inventory metrics
    imb_color = "yellow" if abs(metrics.final_imbalance) > 0.2 else "green"
    table.add_row(
        "Final Imbalance",
        f"[{imb_color}]{metrics.final_imbalance:+.1%}[/{imb_color}]",
    )
    table.add_row("Final Pairs", f"{metrics.final_pairs:.0f}")
    table.add_row("Avg Combined Cost", f"{metrics.avg_combined_cost*100:.1f}c")

    console.print(table)


@app.command("grid-search")
def grid_search(
    config: Annotated[
        Optional[Path],
        typer.Option("--config", "-c", help="Path to YAML config with parameter grid"),
    ] = None,
    data: Annotated[
        Optional[Path],
        typer.Option("--data", "-d", help="Path to market data CSV"),
    ] = None,
    duration: Annotated[
        float,
        typer.Option("--duration", help="Duration in minutes for synthetic data"),
    ] = 15.0,
    seed: Annotated[
        int,
        typer.Option("--seed", "-s", help="Random seed"),
    ] = 42,
    top_n: Annotated[
        int,
        typer.Option("--top-n", "-n", help="Number of top results to display"),
    ] = 10,
    output: Annotated[
        Optional[Path],
        typer.Option("--output", "-o", help="Save results to CSV file"),
    ] = None,
) -> None:
    """Run exhaustive grid search over parameter combinations.

    Tests all combinations of specified parameter values and ranks by performance.
    """
    # Load or generate data
    if data:
        rprint(f"[blue]Loading data from {data}...[/blue]")
        ticks = load_ticks_from_csv(data)
    else:
        rprint(f"[blue]Generating {duration:.1f} minutes of synthetic data...[/blue]")
        ticks = generate_synthetic_ticks(
            duration_minutes=duration,
            random_seed=seed,
        )

    rprint(f"[green]Loaded {len(ticks)} ticks[/green]")

    # Load grid config or use default
    if config:
        with open(config) as f:
            config_dict = yaml.safe_load(f)
        grid = config_dict.get("grid", {})
        fixed = config_dict.get("fixed", {})
    else:
        # Default grid
        grid = {
            "base_spread": [0.01, 0.02, 0.03],
            "gamma_inv": [0.3, 0.5, 0.7, 1.0],
            "oracle_sensitivity": [3.0, 5.0, 10.0],
            "edge_threshold": [0.005, 0.01, 0.02],
        }
        fixed = {}

    # Calculate total combinations
    total_combos = 1
    for values in grid.values():
        total_combos *= len(values)

    # Display grid info
    rprint("\n[bold]Grid Search Configuration[/bold]")
    grid_table = Table(show_header=True, header_style="bold")
    grid_table.add_column("Parameter")
    grid_table.add_column("Values")
    grid_table.add_column("Count", justify="right")

    for param, values in grid.items():
        values_str = ", ".join(f"{v:.3f}" for v in values)
        grid_table.add_row(param, values_str, str(len(values)))

    console.print(grid_table)
    rprint(f"\n[cyan]Total combinations: {total_combos}[/cyan]")

    if fixed:
        rprint("\n[bold]Fixed Parameters:[/bold]")
        for key, value in fixed.items():
            rprint(f"  {key}: {value}")

    rprint("")

    # Run grid search
    backtester = Backtester(
        fill_simulator=FillSimulator(random_seed=seed),
    )
    searcher = GridSearcher(backtester=backtester, ticks=ticks)

    result = searcher.search(
        grid=grid,
        fixed_params=fixed if fixed else None,
        show_progress=True,
    )

    # Display top results
    rprint(f"\n[bold green]Grid Search Complete![/bold green]")
    rprint(f"[cyan]Tested {result.param_combinations} configurations[/cyan]\n")

    df_top = result.top_n(n=top_n, metric="total_pnl", ascending=False)

    rprint(f"[bold]Top {min(top_n, len(df_top))} Results by Total PnL:[/bold]")
    results_table = Table(show_header=True, header_style="bold")

    # Add columns for grid parameters
    for param in grid.keys():
        results_table.add_column(param, justify="right")

    # Add metric columns
    results_table.add_column("Total PnL", justify="right")
    results_table.add_column("Fill Rate", justify="right")
    results_table.add_column("Sharpe", justify="right")
    results_table.add_column("Imbalance", justify="right")

    for _, row in df_top.iterrows():
        values = []
        for param in grid.keys():
            values.append(f"{row[param]:.3f}")

        pnl_color = "green" if row["total_pnl"] > 0 else "red"
        values.append(f"[{pnl_color}]${row['total_pnl']:.2f}[/{pnl_color}]")
        values.append(f"{row['fill_rate']:.1f}%")

        sharpe = row.get("sharpe_ratio")
        sharpe_str = f"{sharpe:.2f}" if sharpe is not None else "N/A"
        values.append(sharpe_str)

        imb_color = "yellow" if abs(row["final_imbalance"]) > 0.2 else "green"
        values.append(f"[{imb_color}]{row['final_imbalance']:+.1%}[/{imb_color}]")

        results_table.add_row(*values)

    console.print(results_table)

    # Summary statistics
    stats = result.summary_stats()
    if stats:
        rprint("\n[bold]Summary Statistics:[/bold]")
        stats_table = Table(show_header=True, header_style="bold")
        stats_table.add_column("Metric")
        stats_table.add_column("Mean", justify="right")
        stats_table.add_column("Std", justify="right")
        stats_table.add_column("Min", justify="right")
        stats_table.add_column("Max", justify="right")

        metric_labels = {
            "total_pnl": "Total PnL",
            "fill_rate": "Fill Rate",
            "sharpe_ratio": "Sharpe Ratio",
            "max_drawdown": "Max Drawdown",
            "final_imbalance": "Final Imbalance",
        }

        for metric, metric_stats in stats.items():
            label = metric_labels.get(metric, metric)
            if metric == "total_pnl" or metric == "max_drawdown":
                stats_table.add_row(
                    label,
                    f"${metric_stats['mean']:.2f}",
                    f"${metric_stats['std']:.2f}",
                    f"${metric_stats['min']:.2f}",
                    f"${metric_stats['max']:.2f}",
                )
            elif metric == "fill_rate":
                stats_table.add_row(
                    label,
                    f"{metric_stats['mean']:.1f}%",
                    f"{metric_stats['std']:.1f}%",
                    f"{metric_stats['min']:.1f}%",
                    f"{metric_stats['max']:.1f}%",
                )
            elif metric == "final_imbalance":
                stats_table.add_row(
                    label,
                    f"{metric_stats['mean']:.1%}",
                    f"{metric_stats['std']:.1%}",
                    f"{metric_stats['min']:.1%}",
                    f"{metric_stats['max']:.1%}",
                )
            else:
                stats_table.add_row(
                    label,
                    f"{metric_stats['mean']:.2f}",
                    f"{metric_stats['std']:.2f}",
                    f"{metric_stats['min']:.2f}",
                    f"{metric_stats['max']:.2f}",
                )

        console.print(stats_table)

    # Save to CSV if requested
    if output:
        df_full = result.to_dataframe()
        df_full.to_csv(output, index=False)
        rprint(f"\n[green]Saved all {len(df_full)} results to {output}[/green]")


@app.command()
def simulate(
    orderbooks: Annotated[
        Path,
        typer.Option("--orderbooks", "-o", help="Path to orderbooks JSON file"),
    ],
    fills: Annotated[
        Path,
        typer.Option("--fills", "-f", help="Path to fills JSON file"),
    ],
    oracle: Annotated[
        Path,
        typer.Option("--oracle", "-r", help="Path to oracle JSON file"),
    ],
    config: Annotated[
        Optional[Path],
        typer.Option("--config", "-c", help="Path to quoter config YAML"),
    ] = None,
    resolution: Annotated[
        Optional[float],
        typer.Option("--resolution", help="Resolution timestamp (Unix)"),
    ] = None,
    output: Annotated[
        Optional[Path],
        typer.Option("--output", help="Save position history to CSV"),
    ] = None,
    graphs: Annotated[
        bool,
        typer.Option("--graphs", "-g", help="Generate simulation report graph"),
    ] = False,
    graph_output: Annotated[
        Optional[Path],
        typer.Option("--graph-output", help="Path for graph output (default: simulation_report.png)"),
    ] = None,
    verbose: Annotated[
        bool,
        typer.Option("--verbose", "-v", help="Verbose output"),
    ] = False,
) -> None:
    """Run simulation against real orderbook data.

    Simulates quoter performance using historical orderbook snapshots,
    fills, and oracle prices from Polymarket.
    """
    from model_tuning.simulation import (
        RealDataSimulator,
        SimulationResult,
        load_simulation_data,
    )

    # Load data
    rprint(f"[blue]Loading orderbooks from {orderbooks}...[/blue]")
    rprint(f"[blue]Loading fills from {fills}...[/blue]")
    rprint(f"[blue]Loading oracle from {oracle}...[/blue]")

    orderbook_data, fill_data, oracle_data = load_simulation_data(
        orderbooks, fills, oracle
    )

    rprint(f"[green]Loaded {len(orderbook_data)} orderbook snapshots[/green]")
    rprint(f"[green]Loaded {len(fill_data)} fills[/green]")
    rprint(f"[green]Loaded {len(oracle_data)} oracle snapshots[/green]")

    # Load quoter config
    if config:
        with open(config) as f:
            config_dict = yaml.safe_load(f)
        params = QuoterParams(**config_dict.get("quoter", {}))
    else:
        params = QuoterParams()

    if verbose:
        rprint("\n[bold]Quoter Parameters:[/bold]")
        for key, value in params.model_dump().items():
            rprint(f"  {key}: {value}")

    # Run simulation
    quoter = InventoryMMQuoter(params)
    simulator = RealDataSimulator()

    rprint("\n[blue]Running simulation...[/blue]")
    result = simulator.run(
        quoter=quoter,
        orderbooks=orderbook_data,
        fills=fill_data,
        oracle=oracle_data,
        resolution_timestamp=resolution,
    )

    # Display results
    _display_simulation_results(result, verbose)

    # Save to CSV if requested
    if output:
        import pandas as pd

        df = pd.DataFrame([ps.model_dump() for ps in result.position_history])
        df.to_csv(output, index=False)
        rprint(f"\n[green]Saved position history to {output}[/green]")

    # Generate graphs if requested
    if graphs:
        from model_tuning.simulation import generate_simulation_report

        graph_path = graph_output or Path("simulation_report.png")
        rprint(f"\n[blue]Generating simulation report graph...[/blue]")
        generate_simulation_report(result, graph_path)
        rprint(f"[green]Saved graph to {graph_path}[/green]")


def _display_simulation_results(result: "SimulationResult", verbose: bool = False) -> None:  # type: ignore[name-defined]
    """Display simulation results in a formatted table."""
    from model_tuning.simulation import SimulationResult

    inv = result.final_inventory

    table = Table(show_header=True, header_style="bold", title="Simulation Results")
    table.add_column("Metric")
    table.add_column("Value", justify="right")

    # Position summary
    table.add_row("UP Position", f"{inv.up_qty:.1f} @ {inv.up_avg:.3f}")
    table.add_row("DOWN Position", f"{inv.down_qty:.1f} @ {inv.down_avg:.3f}")
    table.add_row("Pairs", f"{inv.pairs:.1f}")
    table.add_row("Combined Avg", f"{inv.combined_avg:.3f}")

    # PnL
    pnl_color = "green" if inv.potential_profit > 0 else "red"
    table.add_row(
        "Potential Profit/Pair",
        f"[{pnl_color}]${inv.potential_profit:.4f}[/{pnl_color}]",
    )
    table.add_row(
        "Total Potential PnL",
        f"[{pnl_color}]${result.final_pnl_potential:.2f}[/{pnl_color}]",
    )

    # Fill summary
    table.add_row("Total Fills", f"{result.total_fills}")
    table.add_row("UP Fills", f"{result.up_fills}")
    table.add_row("DOWN Fills", f"{result.down_fills}")
    table.add_row("Total Volume", f"{result.total_volume:.1f}")

    # Imbalance
    imb_color = "yellow" if abs(inv.imbalance) > 0.2 else "green"
    table.add_row(
        "Imbalance",
        f"[{imb_color}]{inv.imbalance:+.1%}[/{imb_color}]",
    )

    console.print(table)

    if verbose and result.matched_fills:
        rprint(
            f"\n[bold]Sample Fills ({min(10, len(result.matched_fills))} "
            f"of {result.total_fills}):[/bold]"
        )
        for mf in result.matched_fills[:10]:
            rprint(
                f"  {mf.outcome.upper()} {mf.size:.1f} @ {mf.price:.3f} "
                f"(market: {mf.original_fill.price:.3f})"
            )


@app.callback()
def main() -> None:
    """Model Tuning CLI for Polymarket quoter optimization."""
    pass


if __name__ == "__main__":
    app()
