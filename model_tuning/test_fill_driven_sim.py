#!/usr/bin/env python3
"""Test script for the fill-driven simulator.

Run with: python test_fill_driven_sim.py

This creates synthetic test data and runs the simulation to verify everything works.
"""

from pathlib import Path

from model_tuning.core.models import Inventory
from model_tuning.simulation import (
    BrainDeadQuoter,
    EnhancedPositionState,
    FillDrivenSimulator,
    OrderbookReconstructor,
    OracleSnapshot,
    RealFill,
    generate_fill_driven_report,
)


def create_test_data():
    """Create synthetic test data for verification."""

    # Raw orderbook data format (as saved by DataFetcher)
    raw_orderbook_data = {
        "up_token_id": "up_token_123",
        "down_token_id": "down_token_456",
        "initial_snapshots": {
            "up_token_123": {
                "timestamp": 1000.0,
                "bids": [
                    {"price": 0.55, "size": 100},
                    {"price": 0.54, "size": 200},
                    {"price": 0.53, "size": 300},
                ],
                "asks": [
                    {"price": 0.57, "size": 100},
                    {"price": 0.58, "size": 200},
                ],
            },
            "down_token_456": {
                "timestamp": 1000.0,
                "bids": [
                    {"price": 0.43, "size": 100},
                    {"price": 0.42, "size": 200},
                    {"price": 0.41, "size": 300},
                ],
                "asks": [
                    {"price": 0.45, "size": 100},
                    {"price": 0.46, "size": 200},
                ],
            },
        },
        "price_changes": [
            # UP token bid increases at t=1005
            {"timestamp": 1005.0, "asset_id": "up_token_123", "price": 0.56, "size": 150, "side": "BUY"},
            # DOWN token bid decreases at t=1010
            {"timestamp": 1010.0, "asset_id": "down_token_456", "price": 0.43, "size": 50, "side": "BUY"},
            # UP token bid moves at t=1020
            {"timestamp": 1020.0, "asset_id": "up_token_123", "price": 0.55, "size": 0, "side": "BUY"},  # Remove
            {"timestamp": 1020.0, "asset_id": "up_token_123", "price": 0.54, "size": 250, "side": "BUY"},
            # More changes
            {"timestamp": 1030.0, "asset_id": "down_token_456", "price": 0.44, "size": 120, "side": "BUY"},
            {"timestamp": 1040.0, "asset_id": "up_token_123", "price": 0.57, "size": 80, "side": "BUY"},
            {"timestamp": 1050.0, "asset_id": "down_token_456", "price": 0.42, "size": 0, "side": "BUY"},  # Remove
        ],
    }

    # Fills - mix of BUY and SELL, UP and DOWN
    # Only SELL fills should match (someone selling into our bid)
    fills = [
        # t=1002: SELL UP at 0.52 - should match (our bid would be 0.55-0.02=0.53, fill at 0.52 <= 0.53)
        RealFill(price=0.52, size=20, side="sell", timestamp=1002.0, outcome="up"),

        # t=1003: BUY UP - should NOT match (we only buy, not sell)
        RealFill(price=0.57, size=30, side="buy", timestamp=1003.0, outcome="up"),

        # t=1007: SELL DOWN at 0.40 - should match (our bid 0.43-0.02=0.41, fill at 0.40 <= 0.41)
        RealFill(price=0.40, size=25, side="sell", timestamp=1007.0, outcome="down"),

        # t=1012: SELL UP at 0.55 - should NOT match (fill price 0.55 > our bid 0.53)
        RealFill(price=0.55, size=15, side="sell", timestamp=1012.0, outcome="up"),

        # t=1015: SELL DOWN at 0.39 - should match
        RealFill(price=0.39, size=30, side="sell", timestamp=1015.0, outcome="down"),

        # t=1022: SELL UP at 0.51 - should match (bid now 0.54-0.02=0.52, fill 0.51 <= 0.52)
        RealFill(price=0.51, size=35, side="sell", timestamp=1022.0, outcome="up"),

        # t=1025: SELL UP at 0.53 - should NOT match (0.53 > 0.52)
        RealFill(price=0.53, size=10, side="sell", timestamp=1025.0, outcome="up"),

        # t=1032: SELL DOWN at 0.41 - should match (bid 0.44-0.02=0.42, fill 0.41 <= 0.42)
        RealFill(price=0.41, size=20, side="sell", timestamp=1032.0, outcome="down"),

        # t=1035: BUY DOWN - should NOT match
        RealFill(price=0.45, size=50, side="buy", timestamp=1035.0, outcome="down"),

        # t=1042: SELL UP at 0.54 - should match (bid 0.57-0.02=0.55, fill 0.54 <= 0.55)
        RealFill(price=0.54, size=40, side="sell", timestamp=1042.0, outcome="up"),

        # t=1045: SELL DOWN at 0.38 - should match (bid still ~0.42)
        RealFill(price=0.38, size=15, side="sell", timestamp=1045.0, outcome="down"),
    ]

    # Oracle data - price fluctuating around threshold
    oracle = [
        OracleSnapshot(price=97000, threshold=97000, timestamp=1000.0),
        OracleSnapshot(price=97100, threshold=97000, timestamp=1005.0),  # Above threshold
        OracleSnapshot(price=97200, threshold=97000, timestamp=1010.0),  # More above
        OracleSnapshot(price=96900, threshold=97000, timestamp=1015.0),  # Below threshold
        OracleSnapshot(price=96800, threshold=97000, timestamp=1020.0),  # More below
        OracleSnapshot(price=97050, threshold=97000, timestamp=1025.0),  # Slightly above
        OracleSnapshot(price=97150, threshold=97000, timestamp=1030.0),  # Above
        OracleSnapshot(price=97300, threshold=97000, timestamp=1040.0),  # Well above
        OracleSnapshot(price=97250, threshold=97000, timestamp=1050.0),  # Still above
    ]

    return raw_orderbook_data, fills, oracle


def run_test():
    """Run the fill-driven simulation test."""

    print("=" * 60)
    print("FILL-DRIVEN SIMULATOR TEST")
    print("=" * 60)

    # Create test data
    print("\n1. Creating synthetic test data...")
    raw_orderbook_data, fills, oracle = create_test_data()
    print(f"   - {len(fills)} fills created")
    print(f"   - {len(oracle)} oracle snapshots")
    print(f"   - {len(raw_orderbook_data['price_changes'])} orderbook price changes")

    # Create reconstructor
    print("\n2. Creating OrderbookReconstructor...")
    reconstructor = OrderbookReconstructor.from_raw_data(raw_orderbook_data)
    print(f"   - Initial timestamp: {reconstructor.initial_timestamp}")
    print(f"   - Final timestamp: {reconstructor.final_timestamp}")

    # Test orderbook reconstruction
    print("\n3. Testing orderbook reconstruction...")
    ob_at_1000 = reconstructor.get_orderbook_at(1000.0)
    print(f"   - At t=1000: UP best_bid={ob_at_1000.up.best_bid}, DOWN best_bid={ob_at_1000.down.best_bid}")

    # Reset and test at different time
    reconstructor = OrderbookReconstructor.from_raw_data(raw_orderbook_data)
    ob_at_1025 = reconstructor.get_orderbook_at(1025.0)
    print(f"   - At t=1025: UP best_bid={ob_at_1025.up.best_bid}, DOWN best_bid={ob_at_1025.down.best_bid}")

    # Create quoter
    print("\n4. Creating BrainDeadQuoter (offset=0.02, size=50)...")
    quoter = BrainDeadQuoter(offset=0.02, size=50.0)

    # Test quote generation
    reconstructor = OrderbookReconstructor.from_raw_data(raw_orderbook_data)
    ob = reconstructor.get_orderbook_at(1001.0)
    quote = quoter.quote(ob)
    print(f"   - Quote at t=1001: bid_up={quote.bid_up}, bid_down={quote.bid_down}, size={quote.size_up}")

    # Run simulation
    print("\n5. Running FillDrivenSimulator...")
    reconstructor = OrderbookReconstructor.from_raw_data(raw_orderbook_data)
    simulator = FillDrivenSimulator()
    result = simulator.run(
        quoter=quoter,
        reconstructor=reconstructor,
        fills=fills,
        oracle=oracle,
    )

    # Print results
    print("\n" + "=" * 60)
    print("SIMULATION RESULTS")
    print("=" * 60)

    print(f"\n📊 Fill Statistics:")
    print(f"   - Total fills in data: {len(fills)}")
    print(f"   - SELL fills considered: {result.total_fills_considered}")
    print(f"   - Fills matched: {result.total_fills_matched}")
    print(f"   - UP fills matched: {result.up_fills}")
    print(f"   - DOWN fills matched: {result.down_fills}")
    print(f"   - Total volume: {result.total_volume}")

    print(f"\n💰 Final Inventory:")
    print(f"   - UP qty: {result.final_inventory.up_qty}")
    print(f"   - DOWN qty: {result.final_inventory.down_qty}")
    print(f"   - UP avg cost: {result.final_inventory.up_avg:.4f}")
    print(f"   - DOWN avg cost: {result.final_inventory.down_avg:.4f}")
    print(f"   - Combined avg: {result.final_inventory.combined_avg:.4f}")
    print(f"   - Pairs: {result.final_inventory.pairs}")

    print(f"\n📈 Final PnL:")
    print(f"   - Merged PnL: ${result.final_merged_pnl:.4f}")
    print(f"   - Directional PnL: ${result.final_directional_pnl:.4f}")
    print(f"   - Total PnL: ${result.final_total_pnl:.4f}")

    print(f"\n📋 Matched Fills Detail:")
    for i, mf in enumerate(result.matched_fills):
        print(f"   {i+1}. t={mf.timestamp}: {mf.outcome.upper()} "
              f"size={mf.size} @ our_bid={mf.price} "
              f"(market sold at {mf.original_fill.price})")

    print(f"\n📉 Position History ({len(result.position_history)} snapshots):")
    for ps in result.position_history:
        print(f"   t={ps.timestamp}: UP={ps.up_qty:.1f} DOWN={ps.down_qty:.1f} "
              f"net={ps.net_qty:.1f} | merged_pnl=${ps.merged_pnl:.2f} "
              f"dir_pnl=${ps.directional_pnl:.2f} total=${ps.total_pnl:.2f}")

    # Generate report
    print("\n6. Generating visualization report...")
    output_path = Path(__file__).parent / "test_simulation_report.png"
    generate_fill_driven_report(result, output_path, title="Test Fill-Driven Simulation")
    print(f"   - Report saved to: {output_path}")

    # Verification checks
    print("\n" + "=" * 60)
    print("VERIFICATION CHECKS")
    print("=" * 60)

    checks_passed = 0
    checks_total = 0

    def check(condition, description):
        nonlocal checks_passed, checks_total
        checks_total += 1
        status = "✅ PASS" if condition else "❌ FAIL"
        print(f"   {status}: {description}")
        if condition:
            checks_passed += 1

    # Check 1: Only SELL fills should be considered
    sell_fills = [f for f in fills if f.side == "sell"]
    check(result.total_fills_considered == len(sell_fills),
          f"Only SELL fills considered ({result.total_fills_considered} == {len(sell_fills)})")

    # Check 2: BUY fills ignored
    buy_fills = [f for f in fills if f.side == "buy"]
    check(len(buy_fills) == 2,
          f"BUY fills exist but ignored ({len(buy_fills)} BUY fills in data)")

    # Check 3: Matched fills have correct side
    check(all(mf.original_fill.side == "sell" for mf in result.matched_fills),
          "All matched fills are SELL fills")

    # Check 4: Fill prices <= our bid
    for mf in result.matched_fills:
        check(mf.original_fill.price <= mf.price,
              f"Fill price {mf.original_fill.price} <= our bid {mf.price}")

    # Check 5: Inventory adds up
    expected_up = sum(mf.size for mf in result.matched_fills if mf.outcome == "up")
    expected_down = sum(mf.size for mf in result.matched_fills if mf.outcome == "down")
    check(result.final_inventory.up_qty == expected_up,
          f"UP inventory matches ({result.final_inventory.up_qty} == {expected_up})")
    check(result.final_inventory.down_qty == expected_down,
          f"DOWN inventory matches ({result.final_inventory.down_qty} == {expected_down})")

    # Check 6: PnL formula
    inv = result.final_inventory
    expected_merged_pnl = inv.pairs * (1 - inv.combined_avg)
    check(abs(result.final_merged_pnl - expected_merged_pnl) < 0.001,
          f"Merged PnL formula correct ({result.final_merged_pnl:.4f} ≈ {expected_merged_pnl:.4f})")

    # Check 7: Total PnL = merged + directional
    check(abs(result.final_total_pnl - (result.final_merged_pnl + result.final_directional_pnl)) < 0.001,
          "Total PnL = Merged + Directional")

    # Check 8: Combined avg < 1 means profit on pairs
    check(inv.combined_avg < 1.0,
          f"Combined avg < 1.0 means profit ({inv.combined_avg:.4f} < 1.0)")

    # Check 9: Position history length matches matched fills
    check(len(result.position_history) == result.total_fills_matched,
          f"Position history length == matched fills ({len(result.position_history)} == {result.total_fills_matched})")

    print(f"\n   Summary: {checks_passed}/{checks_total} checks passed")

    return result


if __name__ == "__main__":
    result = run_test()
    print("\n" + "=" * 60)
    print("TEST COMPLETE - Check test_simulation_report.png for graphs")
    print("=" * 60)
