"""Visualization functions for simulation results.

Generates graphs showing inventory, costs, unrealized PnL, and market prices.
"""

from pathlib import Path

import matplotlib.pyplot as plt

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
