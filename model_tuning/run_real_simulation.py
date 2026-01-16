#!/usr/bin/env python3
"""Complete workflow for running fill-driven simulation on real Polymarket data.

This script guides you through:
1. Fetching live data from a BTC updown market
2. Running the fill-driven simulator
3. Generating the report

Usage:
    # Step 1: Fetch data (run during an active market)
    python run_real_simulation.py fetch <market-slug>

    # Step 2: Run simulation on collected data
    python run_real_simulation.py simulate <market-slug>

    # Or run both (fetch waits for market to end, then simulates)
    python run_real_simulation.py full <market-slug>

To find an active market slug:
    1. Go to https://polymarket.com
    2. Search for "BTC" and find a "15 minute" updown market
    3. The URL will be like: polymarket.com/event/btc-updown-15m-1737012300
    4. The slug is: btc-updown-15m-1737012300
"""

import argparse
import asyncio
import sys
from pathlib import Path

# Add src to path
sys.path.insert(0, str(Path(__file__).parent / "src"))


def fetch_data(slug: str) -> Path:
    """Fetch live data from Polymarket WebSockets.

    Args:
        slug: Market slug (e.g., btc-updown-15m-1737012300)

    Returns:
        Path to the data directory
    """
    from model_tuning.live_data_fetching.fetcher import DataFetcher

    print("=" * 60)
    print("STEP 1: FETCHING LIVE DATA")
    print("=" * 60)
    print(f"Market: {slug}")
    print(f"Output: sim_data/{slug}/")
    print()
    print("This will connect to Polymarket WebSockets and collect:")
    print("  - Fills (trades that occur)")
    print("  - Oracle prices (BTC price from Chainlink)")
    print("  - Orderbook updates (bids/asks)")
    print()
    print("The fetcher will automatically stop 15 seconds before market end.")
    print("Press Ctrl+C to stop early.")
    print()

    fetcher = DataFetcher(slug)

    # Run the fetcher
    asyncio.run(fetcher.connect())

    return fetcher.output_dir


def run_simulation(slug: str) -> None:
    """Run the fill-driven simulator on collected data.

    Args:
        slug: Market slug (same as used for fetching)
    """
    from model_tuning.simulation import (
        BrainDeadQuoter,
        FillDrivenSimulator,
        OrderbookReconstructor,
        load_fills_from_json,
        load_oracle_from_json,
        generate_fill_driven_report,
    )

    print()
    print("=" * 60)
    print("STEP 2: RUNNING SIMULATION")
    print("=" * 60)

    data_dir = Path("sim_data") / slug

    # Check data exists
    if not data_dir.exists():
        print(f"ERROR: Data directory not found: {data_dir}")
        print("Run 'python run_real_simulation.py fetch <slug>' first")
        sys.exit(1)

    orderbook_path = data_dir / "orderbooks_raw.json"
    fills_path = data_dir / "fills.json"
    oracle_path = data_dir / "oracle.json"

    for path, name in [(orderbook_path, "orderbooks_raw.json"),
                       (fills_path, "fills.json"),
                       (oracle_path, "oracle.json")]:
        if not path.exists():
            print(f"ERROR: Missing {name} in {data_dir}")
            sys.exit(1)

    print(f"Loading data from {data_dir}/")

    # Load data
    print("  - Loading orderbook data...")
    reconstructor = OrderbookReconstructor.from_file(orderbook_path)

    print("  - Loading fills...")
    fills = load_fills_from_json(fills_path)

    print("  - Loading oracle data...")
    oracle = load_oracle_from_json(oracle_path)

    print()
    print(f"Data loaded:")
    print(f"  - {len(fills)} fills")
    print(f"  - {len(oracle)} oracle snapshots")
    print(f"  - Orderbook: {reconstructor.initial_timestamp} -> {reconstructor.final_timestamp}")

    # Create quoter
    print()
    print("Creating BrainDeadQuoter (offset=0.02, size=50)...")
    quoter = BrainDeadQuoter(offset=0.02, size=50.0)

    # Run simulation
    print("Running simulation...")
    simulator = FillDrivenSimulator()
    result = simulator.run(
        quoter=quoter,
        reconstructor=reconstructor,
        fills=fills,
        oracle=oracle,
    )

    # Print results
    print()
    print("=" * 60)
    print("SIMULATION RESULTS")
    print("=" * 60)

    print(f"\n📊 Fill Statistics:")
    print(f"   Total fills in data: {len(fills)}")
    print(f"   SELL fills considered: {result.total_fills_considered}")
    print(f"   Fills matched: {result.total_fills_matched}")
    print(f"   UP fills: {result.up_fills}")
    print(f"   DOWN fills: {result.down_fills}")
    print(f"   Total volume: {result.total_volume:.2f}")

    print(f"\n💰 Final Inventory:")
    print(f"   UP qty: {result.final_inventory.up_qty:.2f}")
    print(f"   DOWN qty: {result.final_inventory.down_qty:.2f}")
    print(f"   UP avg cost: ${result.final_inventory.up_avg:.4f}")
    print(f"   DOWN avg cost: ${result.final_inventory.down_avg:.4f}")
    print(f"   Combined avg: ${result.final_inventory.combined_avg:.4f}")
    print(f"   Pairs: {result.final_inventory.pairs:.2f}")

    print(f"\n📈 Final PnL:")
    print(f"   Merged PnL: ${result.final_merged_pnl:.2f}")
    print(f"   Directional PnL: ${result.final_directional_pnl:.2f}")
    print(f"   Total PnL: ${result.final_total_pnl:.2f}")

    # Key metrics
    print(f"\n🎯 Key Metrics:")
    if result.final_inventory.combined_avg < 1.0:
        profit_per_pair = 1.0 - result.final_inventory.combined_avg
        print(f"   Profit per pair: ${profit_per_pair:.4f}")
        print(f"   Status: PROFITABLE on merges ✅")
    else:
        loss_per_pair = result.final_inventory.combined_avg - 1.0
        print(f"   Loss per pair: ${loss_per_pair:.4f}")
        print(f"   Status: UNDERWATER on merges ❌")

    net_position = result.final_inventory.up_qty - result.final_inventory.down_qty
    if abs(net_position) > 0:
        excess_side = "UP" if net_position > 0 else "DOWN"
        print(f"   Directional exposure: {abs(net_position):.2f} {excess_side}")
    else:
        print(f"   Directional exposure: None (perfectly balanced)")

    # Generate report
    print()
    print("Generating report...")
    output_path = data_dir / "simulation_report.png"
    generate_fill_driven_report(result, output_path, title=f"Simulation: {slug}")
    print(f"Report saved to: {output_path}")

    # Also save a text summary
    summary_path = data_dir / "simulation_summary.txt"
    with open(summary_path, "w") as f:
        f.write(f"Simulation Summary: {slug}\n")
        f.write("=" * 60 + "\n\n")
        f.write(f"Fills: {result.total_fills_matched} matched / {result.total_fills_considered} considered\n")
        f.write(f"Volume: {result.total_volume:.2f}\n\n")
        f.write(f"Final Inventory:\n")
        f.write(f"  UP: {result.final_inventory.up_qty:.2f} @ ${result.final_inventory.up_avg:.4f}\n")
        f.write(f"  DOWN: {result.final_inventory.down_qty:.2f} @ ${result.final_inventory.down_avg:.4f}\n")
        f.write(f"  Combined avg: ${result.final_inventory.combined_avg:.4f}\n")
        f.write(f"  Pairs: {result.final_inventory.pairs:.2f}\n\n")
        f.write(f"PnL:\n")
        f.write(f"  Merged: ${result.final_merged_pnl:.2f}\n")
        f.write(f"  Directional: ${result.final_directional_pnl:.2f}\n")
        f.write(f"  Total: ${result.final_total_pnl:.2f}\n")
    print(f"Summary saved to: {summary_path}")


def main():
    parser = argparse.ArgumentParser(
        description="Run fill-driven simulation on real Polymarket data",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # Fetch data from an active market
  python run_real_simulation.py fetch btc-updown-15m-1737012300

  # Run simulation on collected data
  python run_real_simulation.py simulate btc-updown-15m-1737012300

  # Full workflow (fetch + simulate)
  python run_real_simulation.py full btc-updown-15m-1737012300

To find market slugs:
  1. Go to https://polymarket.com
  2. Search for "BTC" updown markets
  3. Look at the URL: polymarket.com/event/<slug>
        """
    )

    subparsers = parser.add_subparsers(dest="command", help="Command to run")

    # Fetch command
    fetch_parser = subparsers.add_parser("fetch", help="Fetch live data from Polymarket")
    fetch_parser.add_argument("slug", help="Market slug (e.g., btc-updown-15m-1737012300)")

    # Simulate command
    sim_parser = subparsers.add_parser("simulate", help="Run simulation on collected data")
    sim_parser.add_argument("slug", help="Market slug (same as used for fetching)")

    # Full command
    full_parser = subparsers.add_parser("full", help="Fetch data then run simulation")
    full_parser.add_argument("slug", help="Market slug")

    args = parser.parse_args()

    if args.command is None:
        parser.print_help()
        sys.exit(1)

    if args.command == "fetch":
        fetch_data(args.slug)
        print()
        print("Data collection complete!")
        print(f"Run 'python run_real_simulation.py simulate {args.slug}' to analyze")

    elif args.command == "simulate":
        run_simulation(args.slug)

    elif args.command == "full":
        fetch_data(args.slug)
        run_simulation(args.slug)


if __name__ == "__main__":
    main()
