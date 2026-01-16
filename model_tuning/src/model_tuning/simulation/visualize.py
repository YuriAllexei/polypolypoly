"""Visualization functions for simulation results.

Generates graphs showing inventory, costs, unrealized PnL, and market prices.
"""

from pathlib import Path

import matplotlib.pyplot as plt

from model_tuning.simulation.fill_driven_simulator import FillDrivenSimulationResult
from model_tuning.simulation.simulator import SimulationResult


def generate_simulation_report(
    result: SimulationResult,
    output_path: Path | str,
) -> None:
    """Generate single figure with all 4 graphs as 2x2 subplots.

    Graphs:
    1. Inventory (UP green, DOWN red) over time
    2. Combined average cost over time
    3. Unrealized PnL from merge mechanism
    4. Best ask prices (UP green, DOWN red) over time

    Args:
        result: SimulationResult from simulator.run()
        output_path: Path to save the PNG file
    """
    if not result.position_history:
        raise ValueError("No position history to plot")

    timestamps = [ps.timestamp for ps in result.position_history]

    fig, axes = plt.subplots(2, 2, figsize=(16, 12))
    fig.suptitle("Simulation Report", fontsize=14, fontweight="bold")

    # 1. Inventory (top-left)
    ax1 = axes[0, 0]
    ax1.plot(
        timestamps,
        [ps.up_qty for ps in result.position_history],
        "g-",
        label="UP",
        linewidth=1.5,
    )
    ax1.plot(
        timestamps,
        [ps.down_qty for ps in result.position_history],
        "r-",
        label="DOWN",
        linewidth=1.5,
    )
    ax1.set_xlabel("Time")
    ax1.set_ylabel("Quantity")
    ax1.set_title("Inventory Over Time")
    ax1.legend()
    ax1.grid(True, alpha=0.3)

    # 2. Combined Avg Cost (top-right)
    ax2 = axes[0, 1]
    combined = [ps.combined_avg for ps in result.position_history]
    ax2.plot(timestamps, combined, "b-", label="Combined Avg", linewidth=1.5)
    ax2.axhline(y=1.0, color="gray", linestyle="--", label="Breakeven", linewidth=1)
    ax2.set_xlabel("Time")
    ax2.set_ylabel("Combined Cost")
    ax2.set_title("Combined Average Cost Over Time")
    ax2.legend()
    ax2.grid(True, alpha=0.3)

    # 3. Unrealized PnL (bottom-left)
    ax3 = axes[1, 0]
    upnl = [ps.pairs * ps.potential_profit for ps in result.position_history]
    ax3.plot(timestamps, upnl, "b-", linewidth=1.5)
    ax3.axhline(y=0, color="gray", linestyle="--", linewidth=1)
    ax3.fill_between(
        timestamps,
        upnl,
        0,
        where=[p >= 0 for p in upnl],
        color="green",
        alpha=0.3,
        label="Profit",
    )
    ax3.fill_between(
        timestamps,
        upnl,
        0,
        where=[p < 0 for p in upnl],
        color="red",
        alpha=0.3,
        label="Loss",
    )
    ax3.set_xlabel("Time")
    ax3.set_ylabel("Unrealized PnL ($)")
    ax3.set_title("Unrealized PnL (Merge Mechanism)")
    ax3.legend()
    ax3.grid(True, alpha=0.3)

    # 4. Best Ask Prices (bottom-right)
    ax4 = axes[1, 1]
    if result.orderbook_history:
        ob_timestamps = [e.timestamp for e in result.orderbook_history]
        ax4.plot(
            ob_timestamps,
            [e.best_ask_up for e in result.orderbook_history],
            "g-",
            label="UP Best Ask",
            linewidth=1.5,
        )
        ax4.plot(
            ob_timestamps,
            [e.best_ask_down for e in result.orderbook_history],
            "r-",
            label="DOWN Best Ask",
            linewidth=1.5,
        )
    ax4.set_xlabel("Time")
    ax4.set_ylabel("Price")
    ax4.set_title("Best Ask Prices Over Time")
    ax4.legend()
    ax4.grid(True, alpha=0.3)

    plt.tight_layout()
    plt.savefig(output_path, dpi=150, bbox_inches="tight")
    plt.close()


def generate_fill_driven_report(
    result: FillDrivenSimulationResult,
    output_path: Path | str,
    title: str | None = None,
) -> None:
    """Generate 4-panel report for fill-driven simulation.

    Panels:
    - Top-Left: Inventory (up_qty green, down_qty red, net_qty blue dashed)
    - Top-Right: Oracle (price blue, threshold red dashed, distance on secondary Y-axis)
    - Bottom-Left: PnL (merged green, directional orange, total blue bold)
    - Bottom-Right: Avg Cost (up_avg green, down_avg red, combined blue, breakeven at 1.0)

    Args:
        result: FillDrivenSimulationResult from simulator.run()
        output_path: Path to save the PNG file
        title: Optional title for the report
    """
    if not result.position_history:
        raise ValueError("No position history to plot")

    timestamps = [ps.timestamp for ps in result.position_history]

    # Convert timestamps to relative minutes from start
    start_ts = timestamps[0]
    rel_minutes = [(ts - start_ts) / 60.0 for ts in timestamps]

    fig, axes = plt.subplots(2, 2, figsize=(16, 12))
    report_title = title or "Fill-Driven Simulation Report"
    fig.suptitle(
        f"{report_title}\n"
        f"Fills: {result.total_fills_matched} matched / {result.total_fills_considered} considered | "
        f"Volume: {result.total_volume:.1f} | "
        f"Final PnL: ${result.final_total_pnl:.2f}",
        fontsize=14,
        fontweight="bold",
    )

    # 1. Inventory (top-left)
    ax1 = axes[0, 0]
    ax1.plot(
        rel_minutes,
        [ps.up_qty for ps in result.position_history],
        "g-",
        label="UP",
        linewidth=1.5,
    )
    ax1.plot(
        rel_minutes,
        [ps.down_qty for ps in result.position_history],
        "r-",
        label="DOWN",
        linewidth=1.5,
    )
    ax1.plot(
        rel_minutes,
        [ps.net_qty for ps in result.position_history],
        "b--",
        label="Net (UP - DOWN)",
        linewidth=1.0,
        alpha=0.7,
    )
    ax1.axhline(y=0, color="gray", linestyle=":", linewidth=0.5)
    ax1.set_xlabel("Time (minutes)")
    ax1.set_ylabel("Quantity")
    ax1.set_title("Inventory Over Time")
    ax1.legend()
    ax1.grid(True, alpha=0.3)

    # 2. Oracle (top-right)
    ax2 = axes[0, 1]
    if result.oracle_history:
        oracle_timestamps = [o.timestamp for o in result.oracle_history]
        oracle_rel_minutes = [(ts - start_ts) / 60.0 for ts in oracle_timestamps]

        # Primary Y-axis: Price and Threshold
        ax2.plot(
            oracle_rel_minutes,
            [o.price for o in result.oracle_history],
            "b-",
            label="Oracle Price",
            linewidth=1.5,
        )
        ax2.plot(
            oracle_rel_minutes,
            [o.threshold for o in result.oracle_history],
            "r--",
            label="Threshold",
            linewidth=1.5,
        )
        ax2.set_xlabel("Time (minutes)")
        ax2.set_ylabel("Price", color="b")
        ax2.tick_params(axis="y", labelcolor="b")
        ax2.legend(loc="upper left")

        # Secondary Y-axis: Distance %
        ax2_twin = ax2.twinx()
        distance_pct = [
            ((o.price - o.threshold) / o.threshold * 100) if o.threshold != 0 else 0.0
            for o in result.oracle_history
        ]
        ax2_twin.plot(
            oracle_rel_minutes,
            distance_pct,
            "g-",
            label="Distance %",
            linewidth=1.0,
            alpha=0.5,
        )
        ax2_twin.axhline(y=0, color="gray", linestyle=":", linewidth=0.5)
        ax2_twin.set_ylabel("Distance from Threshold (%)", color="g")
        ax2_twin.tick_params(axis="y", labelcolor="g")
        ax2_twin.legend(loc="upper right")
    else:
        ax2.text(
            0.5, 0.5, "No Oracle Data",
            ha="center", va="center", transform=ax2.transAxes
        )
    ax2.set_title("Oracle Price vs Threshold")
    ax2.grid(True, alpha=0.3)

    # 3. PnL (bottom-left)
    ax3 = axes[1, 0]
    merged_pnl = [ps.merged_pnl for ps in result.position_history]
    directional_pnl = [ps.directional_pnl for ps in result.position_history]
    total_pnl = [ps.total_pnl for ps in result.position_history]

    ax3.plot(
        rel_minutes,
        merged_pnl,
        "g-",
        label="Merged PnL",
        linewidth=1.5,
    )
    ax3.plot(
        rel_minutes,
        directional_pnl,
        color="orange",
        linestyle="-",
        label="Directional PnL",
        linewidth=1.5,
    )
    ax3.plot(
        rel_minutes,
        total_pnl,
        "b-",
        label="Total PnL",
        linewidth=2.0,
    )
    ax3.axhline(y=0, color="gray", linestyle="--", linewidth=1)

    # Fill between for total PnL
    ax3.fill_between(
        rel_minutes,
        total_pnl,
        0,
        where=[p >= 0 for p in total_pnl],
        color="green",
        alpha=0.2,
    )
    ax3.fill_between(
        rel_minutes,
        total_pnl,
        0,
        where=[p < 0 for p in total_pnl],
        color="red",
        alpha=0.2,
    )

    ax3.set_xlabel("Time (minutes)")
    ax3.set_ylabel("PnL ($)")
    ax3.set_title("Profit & Loss Over Time")
    ax3.legend()
    ax3.grid(True, alpha=0.3)

    # 4. Average Cost (bottom-right)
    ax4 = axes[1, 1]
    ax4.plot(
        rel_minutes,
        [ps.up_avg for ps in result.position_history],
        "g-",
        label="UP Avg Cost",
        linewidth=1.5,
    )
    ax4.plot(
        rel_minutes,
        [ps.down_avg for ps in result.position_history],
        "r-",
        label="DOWN Avg Cost",
        linewidth=1.5,
    )
    ax4.plot(
        rel_minutes,
        [ps.combined_avg for ps in result.position_history],
        "b-",
        label="Combined Avg",
        linewidth=2.0,
    )
    ax4.axhline(
        y=1.0, color="gray", linestyle="--", label="Breakeven", linewidth=1.5
    )
    ax4.set_xlabel("Time (minutes)")
    ax4.set_ylabel("Average Cost")
    ax4.set_title("Average Costs Over Time")
    ax4.legend()
    ax4.grid(True, alpha=0.3)

    plt.tight_layout()
    plt.savefig(output_path, dpi=150, bbox_inches="tight")
    plt.close()
