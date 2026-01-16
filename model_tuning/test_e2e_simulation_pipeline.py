#!/usr/bin/env python3
"""Comprehensive end-to-end test of the fill-driven simulation pipeline.

This test creates mock data files in the exact format DataFetcher produces,
then runs the simulation using the same code path as run_real_simulation.py.
"""

import json
import sys
import tempfile
from pathlib import Path

# Add src to path
sys.path.insert(0, str(Path(__file__).parent / "src"))


def create_mock_data_files(data_dir: Path) -> dict:
    """Create mock data files exactly as DataFetcher would produce them.

    Creates:
    - fills.json: Fill data in RealFill format
    - oracle.json: Oracle price snapshots
    - orderbooks_raw.json: Initial snapshots + price deltas

    Returns:
        Dictionary with metadata about the created data
    """
    # Token IDs (simulating real Polymarket token IDs)
    up_token_id = "0x1234567890abcdef1234567890abcdef12345678"
    down_token_id = "0xfedcba0987654321fedcba0987654321fedcba09"

    # Base timestamp (simulating a market starting)
    base_ts = 1737012300000  # milliseconds

    # === 1. Create fills.json ===
    # Format: {"price": 0.55, "size": 100, "side": "buy", "timestamp": ..., "outcome": "up"}
    fills = [
        # SELL fills (these are the ones we try to match - we buy when others sell)
        {"price": 0.52, "size": 20.0, "side": "sell", "timestamp": base_ts + 2000, "outcome": "up"},
        {"price": 0.40, "size": 25.0, "side": "sell", "timestamp": base_ts + 7000, "outcome": "down"},
        {"price": 0.39, "size": 30.0, "side": "sell", "timestamp": base_ts + 15000, "outcome": "down"},
        {"price": 0.51, "size": 35.0, "side": "sell", "timestamp": base_ts + 22000, "outcome": "up"},
        {"price": 0.53, "size": 10.0, "side": "sell", "timestamp": base_ts + 25000, "outcome": "up"},
        {"price": 0.41, "size": 20.0, "side": "sell", "timestamp": base_ts + 32000, "outcome": "down"},
        {"price": 0.54, "size": 40.0, "side": "sell", "timestamp": base_ts + 42000, "outcome": "up"},
        {"price": 0.38, "size": 15.0, "side": "sell", "timestamp": base_ts + 45000, "outcome": "down"},
        # BUY fills (ignored by our strategy)
        {"price": 0.56, "size": 15.0, "side": "buy", "timestamp": base_ts + 5000, "outcome": "up"},
        {"price": 0.45, "size": 20.0, "side": "buy", "timestamp": base_ts + 35000, "outcome": "down"},
    ]

    with open(data_dir / "fills.json", "w") as f:
        json.dump(fills, f, indent=2)

    # === 2. Create oracle.json ===
    # Format: {"price": 97500.0, "threshold": 98000.0, "timestamp": ...}
    threshold = 98000.0
    oracle = [
        {"price": 97800.0, "threshold": threshold, "timestamp": base_ts},
        {"price": 97750.0, "threshold": threshold, "timestamp": base_ts + 5000},
        {"price": 97600.0, "threshold": threshold, "timestamp": base_ts + 10000},
        {"price": 97550.0, "threshold": threshold, "timestamp": base_ts + 15000},
        {"price": 97700.0, "threshold": threshold, "timestamp": base_ts + 20000},
        {"price": 97850.0, "threshold": threshold, "timestamp": base_ts + 25000},
        {"price": 97900.0, "threshold": threshold, "timestamp": base_ts + 30000},
        {"price": 97950.0, "threshold": threshold, "timestamp": base_ts + 35000},
        {"price": 98050.0, "threshold": threshold, "timestamp": base_ts + 40000},  # Crosses threshold
        {"price": 98100.0, "threshold": threshold, "timestamp": base_ts + 45000},
    ]

    with open(data_dir / "oracle.json", "w") as f:
        json.dump(oracle, f, indent=2)

    # === 3. Create orderbooks_raw.json ===
    # This is the format DataFetcher produces with initial snapshots + deltas
    initial_snapshots = {
        up_token_id: {
            "timestamp": base_ts,
            "bids": [
                {"price": 0.55, "size": 100.0},
                {"price": 0.54, "size": 150.0},
                {"price": 0.53, "size": 200.0},
            ],
            "asks": [
                {"price": 0.56, "size": 100.0},
                {"price": 0.57, "size": 150.0},
                {"price": 0.58, "size": 200.0},
            ],
        },
        down_token_id: {
            "timestamp": base_ts,
            "bids": [
                {"price": 0.43, "size": 100.0},
                {"price": 0.42, "size": 150.0},
                {"price": 0.41, "size": 200.0},
            ],
            "asks": [
                {"price": 0.44, "size": 100.0},
                {"price": 0.45, "size": 150.0},
                {"price": 0.46, "size": 200.0},
            ],
        },
    }

    # Price changes over time (orderbook updates)
    price_changes = [
        # UP side: bid increases slightly
        {"timestamp": base_ts + 10000, "asset_id": up_token_id, "price": 0.56, "size": 120.0, "side": "BUY"},
        {"timestamp": base_ts + 20000, "asset_id": up_token_id, "price": 0.56, "size": 80.0, "side": "BUY"},
        {"timestamp": base_ts + 30000, "asset_id": up_token_id, "price": 0.57, "size": 100.0, "side": "BUY"},
        # DOWN side: bid decreases slightly
        {"timestamp": base_ts + 15000, "asset_id": down_token_id, "price": 0.43, "size": 80.0, "side": "BUY"},
        {"timestamp": base_ts + 25000, "asset_id": down_token_id, "price": 0.44, "size": 90.0, "side": "BUY"},
        {"timestamp": base_ts + 35000, "asset_id": down_token_id, "price": 0.44, "size": 60.0, "side": "BUY"},
        # Some ask changes too
        {"timestamp": base_ts + 40000, "asset_id": up_token_id, "price": 0.58, "size": 50.0, "side": "SELL"},
    ]

    orderbook_raw = {
        "up_token_id": up_token_id,
        "down_token_id": down_token_id,
        "initial_snapshots": initial_snapshots,
        "price_changes": price_changes,
    }

    with open(data_dir / "orderbooks_raw.json", "w") as f:
        json.dump(orderbook_raw, f)

    return {
        "up_token_id": up_token_id,
        "down_token_id": down_token_id,
        "base_ts": base_ts,
        "num_fills": len(fills),
        "num_sell_fills": len([f for f in fills if f["side"] == "sell"]),
        "num_buy_fills": len([f for f in fills if f["side"] == "buy"]),
        "num_oracle": len(oracle),
        "num_price_changes": len(price_changes),
    }


def run_simulation_pipeline(data_dir: Path) -> dict:
    """Run the simulation using the same code path as run_real_simulation.py.

    Returns:
        Dictionary with simulation results
    """
    from model_tuning.simulation import (
        BrainDeadQuoter,
        FillDrivenSimulator,
        OrderbookReconstructor,
        load_fills_from_json,
        load_oracle_from_json,
        generate_fill_driven_report,
    )

    # Load data exactly as run_real_simulation.py does
    orderbook_path = data_dir / "orderbooks_raw.json"
    fills_path = data_dir / "fills.json"
    oracle_path = data_dir / "oracle.json"

    # Verify files exist
    for path, name in [(orderbook_path, "orderbooks_raw.json"),
                       (fills_path, "fills.json"),
                       (oracle_path, "oracle.json")]:
        if not path.exists():
            raise FileNotFoundError(f"Missing {name} in {data_dir}")

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

    # Create quoter (same as run_real_simulation.py)
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

    # Generate report
    print()
    print("Generating report...")
    output_path = data_dir / "simulation_report.png"
    generate_fill_driven_report(result, output_path, title="E2E Test Simulation")
    print(f"Report saved to: {output_path}")

    # Also save text summary (as run_real_simulation.py does)
    summary_path = data_dir / "simulation_summary.txt"
    with open(summary_path, "w") as f:
        f.write("E2E Test Simulation Summary\n")
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

    return {
        "total_fills_considered": result.total_fills_considered,
        "total_fills_matched": result.total_fills_matched,
        "up_fills": result.up_fills,
        "down_fills": result.down_fills,
        "total_volume": result.total_volume,
        "final_up_qty": result.final_inventory.up_qty,
        "final_down_qty": result.final_inventory.down_qty,
        "final_up_avg": result.final_inventory.up_avg,
        "final_down_avg": result.final_inventory.down_avg,
        "combined_avg": result.final_inventory.combined_avg,
        "pairs": result.final_inventory.pairs,
        "merged_pnl": result.final_merged_pnl,
        "directional_pnl": result.final_directional_pnl,
        "total_pnl": result.final_total_pnl,
        "position_history_len": len(result.position_history),
        "report_path": str(output_path),
        "summary_path": str(summary_path),
    }


def verify_results(metadata: dict, results: dict) -> list[tuple[str, bool, str]]:
    """Verify simulation results are correct.

    Returns:
        List of (test_name, passed, message) tuples
    """
    checks = []

    # Check 1: Only SELL fills should be considered
    expected_sell = metadata["num_sell_fills"]
    actual_considered = results["total_fills_considered"]
    passed = actual_considered == expected_sell
    checks.append((
        "SELL fills considered",
        passed,
        f"Expected {expected_sell}, got {actual_considered}"
    ))

    # Check 2: Some fills should be matched (not zero)
    passed = results["total_fills_matched"] > 0
    checks.append((
        "Fills matched > 0",
        passed,
        f"Got {results['total_fills_matched']} matched fills"
    ))

    # Check 3: Position history should have one entry per matched fill
    passed = results["position_history_len"] == results["total_fills_matched"]
    checks.append((
        "Position history length",
        passed,
        f"Expected {results['total_fills_matched']}, got {results['position_history_len']}"
    ))

    # Check 4: UP fills + DOWN fills == total matched
    passed = results["up_fills"] + results["down_fills"] == results["total_fills_matched"]
    checks.append((
        "UP + DOWN == total matched",
        passed,
        f"{results['up_fills']} + {results['down_fills']} == {results['total_fills_matched']}"
    ))

    # Check 5: Volume should be sum of matched fill sizes
    passed = results["total_volume"] > 0
    checks.append((
        "Total volume > 0",
        passed,
        f"Got {results['total_volume']:.2f} volume"
    ))

    # Check 6: Combined avg should be < 1.0 for profit on merges
    passed = results["combined_avg"] < 1.0
    checks.append((
        "Combined avg < 1.0 (profit)",
        passed,
        f"Got {results['combined_avg']:.4f}"
    ))

    # Check 7: Merged PnL formula: pairs * (1 - combined_avg)
    expected_merged = results["pairs"] * (1.0 - results["combined_avg"])
    passed = abs(results["merged_pnl"] - expected_merged) < 0.01
    checks.append((
        "Merged PnL formula correct",
        passed,
        f"Expected {expected_merged:.4f}, got {results['merged_pnl']:.4f}"
    ))

    # Check 8: Total PnL = Merged + Directional
    expected_total = results["merged_pnl"] + results["directional_pnl"]
    passed = abs(results["total_pnl"] - expected_total) < 0.01
    checks.append((
        "Total PnL = Merged + Directional",
        passed,
        f"Expected {expected_total:.4f}, got {results['total_pnl']:.4f}"
    ))

    # Check 9: Report file was created
    report_path = Path(results["report_path"])
    passed = report_path.exists() and report_path.stat().st_size > 0
    checks.append((
        "Report file created",
        passed,
        f"File: {report_path}"
    ))

    # Check 10: Summary file was created
    summary_path = Path(results["summary_path"])
    passed = summary_path.exists() and summary_path.stat().st_size > 0
    checks.append((
        "Summary file created",
        passed,
        f"File: {summary_path}"
    ))

    return checks


def main():
    """Run comprehensive end-to-end test."""
    print("=" * 60)
    print("END-TO-END FILL-DRIVEN SIMULATION PIPELINE TEST")
    print("=" * 60)
    print()

    # Use a temporary directory for test data
    with tempfile.TemporaryDirectory() as tmpdir:
        data_dir = Path(tmpdir) / "test-market-slug"
        data_dir.mkdir(parents=True)

        print("1. Creating mock data files...")
        print(f"   Directory: {data_dir}")
        metadata = create_mock_data_files(data_dir)
        print(f"   - {metadata['num_fills']} fills ({metadata['num_sell_fills']} SELL, {metadata['num_buy_fills']} BUY)")
        print(f"   - {metadata['num_oracle']} oracle snapshots")
        print(f"   - {metadata['num_price_changes']} price changes")
        print()

        # Verify files were created
        for filename in ["fills.json", "oracle.json", "orderbooks_raw.json"]:
            filepath = data_dir / filename
            if filepath.exists():
                print(f"   [OK] {filename} created ({filepath.stat().st_size} bytes)")
            else:
                print(f"   [FAIL] {filename} NOT created")
                return 1
        print()

        print("2. Running simulation pipeline...")
        print("-" * 60)
        try:
            results = run_simulation_pipeline(data_dir)
        except Exception as e:
            print(f"\n[FATAL] Simulation failed: {e}")
            import traceback
            traceback.print_exc()
            return 1
        print("-" * 60)
        print()

        print("3. Simulation Results:")
        print(f"   Total fills considered: {results['total_fills_considered']}")
        print(f"   Total fills matched: {results['total_fills_matched']}")
        print(f"   UP fills: {results['up_fills']}")
        print(f"   DOWN fills: {results['down_fills']}")
        print(f"   Total volume: {results['total_volume']:.2f}")
        print()
        print(f"   Final UP qty: {results['final_up_qty']:.2f} @ ${results['final_up_avg']:.4f}")
        print(f"   Final DOWN qty: {results['final_down_qty']:.2f} @ ${results['final_down_avg']:.4f}")
        print(f"   Combined avg: ${results['combined_avg']:.4f}")
        print(f"   Pairs: {results['pairs']:.2f}")
        print()
        print(f"   Merged PnL: ${results['merged_pnl']:.2f}")
        print(f"   Directional PnL: ${results['directional_pnl']:.2f}")
        print(f"   Total PnL: ${results['total_pnl']:.2f}")
        print()

        print("4. Verification Checks:")
        print("-" * 60)
        checks = verify_results(metadata, results)

        passed_count = 0
        failed_count = 0
        for test_name, passed, message in checks:
            status = "PASS" if passed else "FAIL"
            symbol = "[OK]" if passed else "[FAIL]"
            print(f"   {symbol} {test_name}: {message}")
            if passed:
                passed_count += 1
            else:
                failed_count += 1

        print("-" * 60)
        print(f"   Summary: {passed_count}/{passed_count + failed_count} checks passed")
        print()

        # Copy report to persistent location before temp dir is cleaned up
        persistent_report = Path(__file__).parent / "e2e_test_report.png"
        import shutil
        shutil.copy(results["report_path"], persistent_report)
        print(f"Report copied to: {persistent_report}")
        print()

        if failed_count > 0:
            print("=" * 60)
            print("TEST FAILED")
            print("=" * 60)
            return 1
        else:
            print("=" * 60)
            print("ALL TESTS PASSED")
            print("=" * 60)
            return 0


if __name__ == "__main__":
    sys.exit(main())
