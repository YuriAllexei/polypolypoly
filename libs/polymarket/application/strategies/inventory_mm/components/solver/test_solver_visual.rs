//! Visual test for solver - run with `cargo test test_solver_visual -- --nocapture`

#[cfg(test)]
mod visual_tests {
    use crate::application::strategies::inventory_mm::components::solver::solve;
    use crate::application::strategies::inventory_mm::types::{
        SolverInput, SolverConfig, InventorySnapshot, OrderbookSnapshot, OrderSnapshot, OpenOrder,
    };

    fn print_separator(title: &str) {
        println!("\n{}", "=".repeat(70));
        println!("  {}", title);
        println!("{}", "=".repeat(70));
    }

    fn print_input(input: &SolverInput) {
        let delta = input.inventory.imbalance();
        println!("\n--- INPUT ---");
        println!("Inventory:");
        println!("  UP:   {} @ ${:.4} avg", input.inventory.up_size, input.inventory.up_avg_price);
        println!("  DOWN: {} @ ${:.4} avg", input.inventory.down_size, input.inventory.down_avg_price);
        println!("  Delta (imbalance): {:.4} [{:.0}% UP / {:.0}% DOWN]",
            delta,
            (1.0 + delta) / 2.0 * 100.0,
            (1.0 - delta) / 2.0 * 100.0
        );
        println!("  Combined avg cost: ${:.4}", input.inventory.combined_avg_cost());
        println!("  Pairs available for merge: {:.1}", input.inventory.pairs_available());

        println!("\nOrderbooks:");
        if let Some((ask, size)) = input.up_orderbook.best_ask {
            println!("  UP   best_ask: ${:.2} ({} size)", ask, size);
        }
        if let Some((bid, size)) = input.up_orderbook.best_bid {
            println!("  UP   best_bid: ${:.2} ({} size)", bid, size);
        }
        if let Some((ask, size)) = input.down_orderbook.best_ask {
            println!("  DOWN best_ask: ${:.2} ({} size)", ask, size);
        }
        if let Some((bid, size)) = input.down_orderbook.best_bid {
            println!("  DOWN best_bid: ${:.2} ({} size)", bid, size);
        }

        println!("\nExisting Orders:");
        if input.up_orders.bids.is_empty() && input.down_orders.bids.is_empty() {
            println!("  (none)");
        }
        for o in &input.up_orders.bids {
            println!("  UP   BID: ${:.2} x {} (id: {})", o.price, o.remaining_size, o.order_id);
        }
        for o in &input.down_orders.bids {
            println!("  DOWN BID: ${:.2} x {} (id: {})", o.price, o.remaining_size, o.order_id);
        }

        println!("\nConfig:");
        println!("  num_levels: {}", input.config.num_levels);
        println!("  base_offset: ${:.2}", input.config.base_offset);
        println!("  spread_per_level: {} cents", input.config.spread_per_level);
        println!("  min_profit_margin: ${:.2}", input.config.min_profit_margin);
        println!("  max_imbalance: {:.0}%", input.config.max_imbalance * 100.0);
        println!("  order_size: {}", input.config.order_size);
    }

    fn print_output(input: &SolverInput, output: &crate::application::strategies::inventory_mm::types::SolverOutput) {
        println!("\n--- OUTPUT ---");

        let delta = input.inventory.imbalance();

        println!("\nOffset Calculation (based on delta={:.4}, scaling={:.1}):", delta, input.config.offset_scaling);
        let up_offset = input.config.base_offset * (1.0 + delta.max(0.0) * input.config.offset_scaling);
        let down_offset = input.config.base_offset * (1.0 + (-delta).max(0.0) * input.config.offset_scaling);
        println!("  UP offset:   ${:.4} (base ${:.2} * {:.2})", up_offset, input.config.base_offset, 1.0 + delta.max(0.0) * input.config.offset_scaling);
        println!("  DOWN offset: ${:.4} (base ${:.2} * {:.2})", down_offset, input.config.base_offset, 1.0 + (-delta).max(0.0) * input.config.offset_scaling);

        if !output.limit_orders.is_empty() {
            println!("\nLimit Orders to PLACE:");
            let up_orders: Vec<_> = output.limit_orders.iter().filter(|o| o.token_id.contains("up")).collect();
            let down_orders: Vec<_> = output.limit_orders.iter().filter(|o| o.token_id.contains("down")).collect();

            if !up_orders.is_empty() {
                println!("  UP bids:");
                for o in up_orders {
                    println!("    ${:.2} x {} ({:?})", o.price, o.size, o.side);
                }
            } else {
                println!("  UP bids: (none - delta {:.2} >= max_imbalance {:.2})", delta, input.config.max_imbalance);
            }

            if !down_orders.is_empty() {
                println!("  DOWN bids:");
                for o in down_orders {
                    println!("    ${:.2} x {} ({:?})", o.price, o.size, o.side);
                }
            } else {
                println!("  DOWN bids: (none - delta {:.2} <= -{:.2})", delta, input.config.max_imbalance);
            }
        } else {
            println!("\nLimit Orders to PLACE: (none)");
        }

        if !output.cancellations.is_empty() {
            println!("\nOrders to CANCEL:");
            for id in &output.cancellations {
                println!("  {}", id);
            }
        }

        if !output.taker_orders.is_empty() {
            println!("\nTaker Orders (immediate execution):");
            for t in &output.taker_orders {
                println!("  {:?} {} @ ${:.2} x {}", t.side, t.token_id, t.price, t.size);
            }
        }

        println!("\nSummary: {} cancels, {} limit orders, {} takers",
            output.cancellations.len(),
            output.limit_orders.len(),
            output.taker_orders.len()
        );
    }

    fn make_config() -> SolverConfig {
        SolverConfig {
            num_levels: 3,
            tick_size: 0.01,
            base_offset: 0.01,
            min_profit_margin: 0.01,
            max_imbalance: 0.8,
            order_size: 100.0,
            spread_per_level: 1.0,
            offset_scaling: 5.0,
        }
    }

    #[test]
    fn test_solver_visual() {
        // =====================================================================
        // SCENARIO 1: Balanced inventory, no existing orders
        // =====================================================================
        print_separator("SCENARIO 1: Balanced Inventory (50/50), No Existing Orders");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.46,
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.55, 500.0)),
                best_bid: Some((0.53, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.45, 500.0)),
                best_bid: Some((0.43, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // =====================================================================
        // SCENARIO 2: Heavy UP inventory (need more DOWN)
        // =====================================================================
        print_separator("SCENARIO 2: Heavy UP Inventory (80/20), Need More DOWN");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 80.0,
                up_avg_price: 0.52,
                down_size: 20.0,
                down_avg_price: 0.46,
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.55, 500.0)),
                best_bid: Some((0.53, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.45, 500.0)),
                best_bid: Some((0.43, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // =====================================================================
        // SCENARIO 3: Extreme imbalance (should stop UP quotes)
        // =====================================================================
        print_separator("SCENARIO 3: Extreme UP Imbalance (95/5), Should STOP UP Quotes");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 95.0,
                up_avg_price: 0.52,
                down_size: 5.0,
                down_avg_price: 0.46,
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.55, 500.0)),
                best_bid: Some((0.53, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.45, 500.0)),
                best_bid: Some((0.43, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // =====================================================================
        // SCENARIO 4: Heavy DOWN inventory
        // =====================================================================
        print_separator("SCENARIO 4: Heavy DOWN Inventory (20/80), Need More UP");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 20.0,
                up_avg_price: 0.52,
                down_size: 80.0,
                down_avg_price: 0.46,
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.55, 500.0)),
                best_bid: Some((0.53, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.45, 500.0)),
                best_bid: Some((0.43, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // =====================================================================
        // SCENARIO 5: Empty inventory (fresh start)
        // =====================================================================
        print_separator("SCENARIO 5: Empty Inventory (Fresh Start)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 0.0,
                up_avg_price: 0.0,
                down_size: 0.0,
                down_avg_price: 0.0,
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.55, 500.0)),
                best_bid: Some((0.53, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.45, 500.0)),
                best_bid: Some((0.43, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // =====================================================================
        // SCENARIO 6: Existing orders that need updating
        // =====================================================================
        print_separator("SCENARIO 6: Existing Orders - Stale Prices Need Refresh");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::new("up-order-1".to_string(), 0.50, 100.0, 100.0), // stale - too low
                    OpenOrder::new("up-order-2".to_string(), 0.51, 100.0, 100.0), // stale
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::new("down-order-1".to_string(), 0.40, 100.0, 100.0), // stale
                ],
                asks: vec![],
            },
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.46,
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.55, 500.0)),
                best_bid: Some((0.53, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.45, 500.0)),
                best_bid: Some((0.43, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // =====================================================================
        // SCENARIO 7: Unprofitable inventory (high combined cost)
        // =====================================================================
        print_separator("SCENARIO 7: Unprofitable Inventory (Combined Cost = $1.01)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.55,  // high
                down_size: 50.0,
                down_avg_price: 0.46,  // combined = 1.01
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.55, 500.0)),
                best_bid: Some((0.53, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.45, 500.0)),
                best_bid: Some((0.43, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // =====================================================================
        // SCENARIO 8: Tight market (low spread)
        // =====================================================================
        print_separator("SCENARIO 8: Tight Market (Small Spread)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.46,
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.54, 500.0)),  // tight: only 1 cent from bid
                best_bid: Some((0.53, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.47, 500.0)),  // tight
                best_bid: Some((0.46, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // =====================================================================
        // SCENARIO 9: Heavy DOWN with cheap UP (should generate UP taker)
        // =====================================================================
        print_separator("SCENARIO 9: Heavy DOWN + Cheap UP Ask (Should Take UP)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 20.0,
                up_avg_price: 0.52,
                down_size: 80.0,
                down_avg_price: 0.46,
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.50, 500.0)),  // Cheap UP!
                best_bid: Some((0.48, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.50, 500.0)),
                best_bid: Some((0.48, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // Explain the taker math
        println!("\n--- TAKER CALCULATION ---");
        println!("Need UP (delta={:.2}), checking UP ask at $0.50", input.inventory.imbalance());
        let old_cost = 20.0 * 0.52;
        let new_cost = 100.0 * 0.50;
        let new_up_avg = (old_cost + new_cost) / 120.0;
        let combined = new_up_avg + 0.46;
        println!("  old_up_cost = 20 * $0.52 = ${:.2}", old_cost);
        println!("  new_up_cost = 100 * $0.50 = ${:.2}", new_cost);
        println!("  new_up_avg = ${:.2} / 120 = ${:.4}", old_cost + new_cost, new_up_avg);
        println!("  combined = ${:.4} + $0.46 = ${:.4}", new_up_avg, combined);
        println!("  threshold = $0.99");
        println!("  {} < $0.99? {} → {}",
            combined,
            combined < 0.99,
            if combined < 0.99 { "TAKE!" } else { "NO TAKE" }
        );

        println!("\n{}", "=".repeat(70));
        println!("  END OF VISUAL TESTS");
        println!("{}\n", "=".repeat(70));
    }
}
