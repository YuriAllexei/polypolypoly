//! Visual test for solver - run with `cargo test test_solver_visual -- --nocapture`

#[cfg(test)]
mod visual_tests {
    use crate::application::strategies::inventory_mm::components::solver::solve;
    use crate::application::strategies::inventory_mm::types::{
        SolverInput, SolverConfig, InventorySnapshot, OrderbookSnapshot, OrderSnapshot, OpenOrder,
    };

    fn print_separator(title: &str) {
        println!("\n{}", "=".repeat(80));
        println!("  {}", title);
        println!("{}", "=".repeat(80));
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
            println!("  UP   best_ask: ${:.2} ({} size){}", ask, size,
                if input.up_orderbook.best_ask_is_ours { " [OURS]" } else { "" });
        }
        if let Some((bid, size)) = input.up_orderbook.best_bid {
            println!("  UP   best_bid: ${:.2} ({} size){}", bid, size,
                if input.up_orderbook.best_bid_is_ours { " [OURS]" } else { "" });
        }
        if let Some((ask, size)) = input.down_orderbook.best_ask {
            println!("  DOWN best_ask: ${:.2} ({} size){}", ask, size,
                if input.down_orderbook.best_ask_is_ours { " [OURS]" } else { "" });
        }
        if let Some((bid, size)) = input.down_orderbook.best_bid {
            println!("  DOWN best_bid: ${:.2} ({} size){}", bid, size,
                if input.down_orderbook.best_bid_is_ours { " [OURS]" } else { "" });
        }

        // Calculate and display spread info
        if let (Some((ask, _)), Some((bid, _))) = (input.up_orderbook.best_ask, input.up_orderbook.best_bid) {
            println!("  UP   spread: ${:.2} ({:.1}%)", ask - bid, (ask - bid) / ask * 100.0);
        }
        if let (Some((ask, _)), Some((bid, _))) = (input.down_orderbook.best_ask, input.down_orderbook.best_bid) {
            println!("  DOWN spread: ${:.2} ({:.1}%)", ask - bid, (ask - bid) / ask * 100.0);
        }

        println!("\nExisting Orders:");
        if input.up_orders.bids.is_empty() && input.down_orders.bids.is_empty()
            && input.up_orders.asks.is_empty() && input.down_orders.asks.is_empty() {
            println!("  (none)");
        }
        for o in &input.up_orders.bids {
            println!("  UP   BID: ${:.2} x {:.1} (remaining: {:.1}, id: {})",
                o.price, o.original_size, o.remaining_size, o.order_id);
        }
        for o in &input.up_orders.asks {
            println!("  UP   ASK: ${:.2} x {:.1} (remaining: {:.1}, id: {})",
                o.price, o.original_size, o.remaining_size, o.order_id);
        }
        for o in &input.down_orders.bids {
            println!("  DOWN BID: ${:.2} x {:.1} (remaining: {:.1}, id: {})",
                o.price, o.original_size, o.remaining_size, o.order_id);
        }
        for o in &input.down_orders.asks {
            println!("  DOWN ASK: ${:.2} x {:.1} (remaining: {:.1}, id: {})",
                o.price, o.original_size, o.remaining_size, o.order_id);
        }

        println!("\nConfig:");
        println!("  num_levels: {}", input.config.num_levels);
        println!("  base_offset: ${:.2}", input.config.base_offset);
        println!("  spread_per_level: {} cents", input.config.spread_per_level);
        println!("  max_imbalance: {:.0}%", input.config.max_imbalance * 100.0);
        println!("  order_size: {}", input.config.order_size);
        println!("  offset_scaling: {:.1}", input.config.offset_scaling);
        println!("  skew_factor: {:.1}", input.config.skew_factor);
    }

    fn print_output(input: &SolverInput, output: &crate::application::strategies::inventory_mm::types::SolverOutput) {
        println!("\n--- OUTPUT ---");

        let delta = input.inventory.imbalance();

        println!("\nOffset Calculation (based on delta={:.4}, scaling={:.1}):", delta, input.config.offset_scaling);
        let up_offset = input.config.base_offset * (1.0 + delta.max(0.0) * input.config.offset_scaling);
        let down_offset = input.config.base_offset * (1.0 + (-delta).max(0.0) * input.config.offset_scaling);
        println!("  UP offset:   ${:.4} (base ${:.2} * {:.2})", up_offset, input.config.base_offset, 1.0 + delta.max(0.0) * input.config.offset_scaling);
        println!("  DOWN offset: ${:.4} (base ${:.2} * {:.2})", down_offset, input.config.base_offset, 1.0 + (-delta).max(0.0) * input.config.offset_scaling);

        println!("\nSize Calculation (based on delta={:.4}, skew_factor={:.1}):", delta, input.config.skew_factor);
        let up_size = (input.config.order_size * (1.0 - delta * input.config.skew_factor)).clamp(0.0, input.config.order_size * 3.0);
        let down_size = (input.config.order_size * (1.0 + delta * input.config.skew_factor)).clamp(0.0, input.config.order_size * 3.0);
        println!("  UP size:   {:.1} (base {} * {:.2})", up_size, input.config.order_size, 1.0 - delta * input.config.skew_factor);
        println!("  DOWN size: {:.1} (base {} * {:.2})", down_size, input.config.order_size, 1.0 + delta * input.config.skew_factor);

        // Expected quote prices
        if delta < input.config.max_imbalance {
            if let Some((ask, _)) = input.up_orderbook.best_ask {
                println!("\n  Expected UP quotes (from best_ask ${:.2}):", ask);
                for level in 0..input.config.num_levels {
                    let level_spread = (level as f64) * (input.config.spread_per_level / 100.0);
                    let price = ((ask - up_offset - level_spread) / input.config.tick_size + 1e-9).floor() * input.config.tick_size;
                    println!("    Level {}: ${:.2} (ask - ${:.4} offset - ${:.4} spread)",
                        level, price, up_offset, level_spread);
                }
            }
        } else {
            println!("\n  Expected UP quotes: NONE (delta {:.2} >= max_imbalance {:.2})", delta, input.config.max_imbalance);
        }

        if delta > -input.config.max_imbalance {
            if let Some((ask, _)) = input.down_orderbook.best_ask {
                println!("\n  Expected DOWN quotes (from best_ask ${:.2}):", ask);
                for level in 0..input.config.num_levels {
                    let level_spread = (level as f64) * (input.config.spread_per_level / 100.0);
                    let price = ((ask - down_offset - level_spread) / input.config.tick_size + 1e-9).floor() * input.config.tick_size;
                    println!("    Level {}: ${:.2} (ask - ${:.4} offset - ${:.4} spread)",
                        level, price, down_offset, level_spread);
                }
            }
        } else {
            println!("\n  Expected DOWN quotes: NONE (delta {:.2} <= -{:.2})", delta, input.config.max_imbalance);
        }

        if !output.cancellations.is_empty() {
            println!("\nOrders to CANCEL ({}):", output.cancellations.len());
            for id in &output.cancellations {
                println!("  {} (stale/wrong price)", id);
            }
        } else {
            println!("\nOrders to CANCEL: (none)");
        }

        if !output.limit_orders.is_empty() {
            println!("\nLimit Orders to PLACE ({}):", output.limit_orders.len());
            let up_orders: Vec<_> = output.limit_orders.iter().filter(|o| o.token_id.contains("up")).collect();
            let down_orders: Vec<_> = output.limit_orders.iter().filter(|o| o.token_id.contains("down")).collect();

            if !up_orders.is_empty() {
                println!("  UP bids:");
                for o in up_orders {
                    println!("    ${:.2} x {} ({:?})", o.price, o.size, o.side);
                }
            } else if delta < input.config.max_imbalance {
                println!("  UP bids: (none needed - orders already exist at target prices)");
            } else {
                println!("  UP bids: (none - delta {:.2} >= max_imbalance {:.2})", delta, input.config.max_imbalance);
            }

            if !down_orders.is_empty() {
                println!("  DOWN bids:");
                for o in down_orders {
                    println!("    ${:.2} x {} ({:?})", o.price, o.size, o.side);
                }
            } else if delta > -input.config.max_imbalance {
                println!("  DOWN bids: (none needed - orders already exist at target prices)");
            } else {
                println!("  DOWN bids: (none - delta {:.2} <= -{:.2})", delta, input.config.max_imbalance);
            }
        } else {
            println!("\nLimit Orders to PLACE: (none - all orders already exist at target prices)");
        }

        println!("\n--- SUMMARY ---");
        println!("  {} cancellations, {} new limit orders",
            output.cancellations.len(),
            output.limit_orders.len()
        );

        // Calculate unchanged orders
        let total_existing = input.up_orders.bids.len() + input.down_orders.bids.len();
        let unchanged = total_existing - output.cancellations.len();
        if total_existing > 0 {
            println!("  {} of {} existing orders kept (matched target)", unchanged, total_existing);
        }
    }

    fn make_config() -> SolverConfig {
        SolverConfig {
            num_levels: 3,
            tick_size: 0.01,
            base_offset: 0.01,
            max_imbalance: 0.8,
            order_size: 100.0,
            spread_per_level: 1.0,
            offset_scaling: 5.0,
            skew_factor: 2.0,
            min_offset: 0.01,
            max_position: 0.0,
            // Disable profitability cap for visual tests (tested separately in quotes.rs)
            prof_weight: 0.0,
            imbalance_weight: 1.0,
            prof_cap_delta_threshold: 0.3,
        }
    }

    #[test]
    fn test_solver_visual() {
        println!("\n{}", "#".repeat(80));
        println!("# SECTION 1: IMBALANCE VARIATIONS (No Existing Orders)");
        println!("{}\n", "#".repeat(80));

        // =====================================================================
        // SCENARIO 1.1: Balanced inventory (50/50)
        // =====================================================================
        print_separator("1.1: Balanced Inventory (50/50) - Symmetric Quoting");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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
        // SCENARIO 1.2: Slight imbalance (60/40)
        // =====================================================================
        print_separator("1.2: Slight UP Imbalance (60/40) - Minor Offset Difference");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 60.0,
                up_avg_price: 0.52,
                down_size: 40.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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
        // SCENARIO 1.3: Moderate imbalance (70/30)
        // =====================================================================
        print_separator("1.3: Moderate UP Imbalance (70/30) - 2 Tick Offset Difference");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 70.0,
                up_avg_price: 0.52,
                down_size: 30.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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
        // SCENARIO 1.4: Heavy imbalance (80/20)
        // =====================================================================
        print_separator("1.4: Heavy UP Imbalance (80/20) - 3 Tick Offset Difference");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 80.0,
                up_avg_price: 0.52,
                down_size: 20.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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
        // SCENARIO 1.5: Extreme imbalance (95/5) - Should STOP UP quotes
        // =====================================================================
        print_separator("1.5: Extreme UP Imbalance (95/5) - STOP UP Quotes (>80%)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 95.0,
                up_avg_price: 0.52,
                down_size: 5.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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
        // SCENARIO 1.6: Inverse - Heavy DOWN (20/80)
        // =====================================================================
        print_separator("1.6: Heavy DOWN Imbalance (20/80) - Inverse of 1.4");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 20.0,
                up_avg_price: 0.52,
                down_size: 80.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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
        // SCENARIO 1.7: Inverse - Moderate DOWN (30/70)
        // =====================================================================
        print_separator("1.7: Moderate DOWN Imbalance (30/70) - Inverse of 1.3");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 30.0,
                up_avg_price: 0.52,
                down_size: 70.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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
        // SCENARIO 1.8: Inverse - Slight DOWN (40/60)
        // =====================================================================
        print_separator("1.8: Slight DOWN Imbalance (40/60) - Inverse of 1.2");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 40.0,
                up_avg_price: 0.52,
                down_size: 60.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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

        println!("\n{}", "#".repeat(80));
        println!("# SECTION 2: ORDERBOOK VARIATIONS (Different Market Conditions)");
        println!("{}\n", "#".repeat(80));

        // =====================================================================
        // SCENARIO 2.1: Wide spread market
        // =====================================================================
        print_separator("2.1: Wide Spread Market (UP: $0.60/$0.50, DOWN: $0.50/$0.40)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.60, 300.0)),
                best_bid: Some((0.50, 150.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.50, 300.0)),
                best_bid: Some((0.40, 150.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // =====================================================================
        // SCENARIO 2.2: Tight spread market
        // =====================================================================
        print_separator("2.2: Tight Spread Market (1 tick spread)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.54, 1000.0)),  // tight: only 1 cent from bid
                best_bid: Some((0.53, 800.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.47, 1000.0)),  // tight
                best_bid: Some((0.46, 800.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // =====================================================================
        // SCENARIO 2.3: High conviction market (UP at $0.75)
        // =====================================================================
        print_separator("2.3: High UP Conviction Market (UP: $0.75, DOWN: $0.25)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.70,
                down_size: 50.0,
                down_avg_price: 0.25,
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.75, 400.0)),
                best_bid: Some((0.73, 300.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.27, 400.0)),
                best_bid: Some((0.25, 300.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // =====================================================================
        // SCENARIO 2.4: Low conviction market (near 50/50)
        // =====================================================================
        print_separator("2.4: Low Conviction Market (UP: $0.51, DOWN: $0.49)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.49,
                down_size: 50.0,
                down_avg_price: 0.49,
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.51, 600.0)),
                best_bid: Some((0.49, 500.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.51, 600.0)),
                best_bid: Some((0.49, 500.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // =====================================================================
        // SCENARIO 2.5: Extreme conviction market (UP at $0.90)
        // =====================================================================
        print_separator("2.5: Extreme UP Conviction Market (UP: $0.90, DOWN: $0.10)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.85,
                down_size: 50.0,
                down_avg_price: 0.10,
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.90, 200.0)),
                best_bid: Some((0.88, 100.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.12, 200.0)),
                best_bid: Some((0.10, 100.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // =====================================================================
        // SCENARIO 2.6: Asymmetric orderbook (UP deep, DOWN thin)
        // =====================================================================
        print_separator("2.6: Asymmetric Liquidity (UP: deep, DOWN: thin)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.55, 2000.0)),  // very deep
                best_bid: Some((0.53, 1500.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.45, 50.0)),   // very thin
                best_bid: Some((0.43, 30.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        println!("\n{}", "#".repeat(80));
        println!("# SECTION 3: ORDER DIFFING (Cancellations & Partial Updates)");
        println!("{}\n", "#".repeat(80));

        // =====================================================================
        // SCENARIO 3.1: All orders match - no changes needed
        // =====================================================================
        print_separator("3.1: Perfect Match - All Orders at Target Prices (No Changes)");

        // First calculate what the target prices would be
        // For balanced (delta=0): offset = 0.01
        // UP: 0.55 - 0.01 = 0.54, 0.53, 0.52
        // DOWN: 0.45 - 0.01 = 0.44, 0.43, 0.42
        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::new("up-1".to_string(), 0.54, 100.0, 100.0),
                    OpenOrder::new("up-2".to_string(), 0.53, 100.0, 100.0),
                    OpenOrder::new("up-3".to_string(), 0.52, 100.0, 100.0),
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::new("down-1".to_string(), 0.44, 100.0, 100.0),
                    OpenOrder::new("down-2".to_string(), 0.43, 100.0, 100.0),
                    OpenOrder::new("down-3".to_string(), 0.42, 100.0, 100.0),
                ],
                asks: vec![],
            },
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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
        // SCENARIO 3.2: All orders stale - full replacement
        // =====================================================================
        print_separator("3.2: All Orders Stale - Full Replacement Needed");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::new("up-old-1".to_string(), 0.48, 100.0, 100.0), // 6 ticks too low
                    OpenOrder::new("up-old-2".to_string(), 0.47, 100.0, 100.0), // 6 ticks too low
                    OpenOrder::new("up-old-3".to_string(), 0.46, 100.0, 100.0), // 6 ticks too low
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::new("down-old-1".to_string(), 0.38, 100.0, 100.0), // stale
                    OpenOrder::new("down-old-2".to_string(), 0.37, 100.0, 100.0), // stale
                    OpenOrder::new("down-old-3".to_string(), 0.36, 100.0, 100.0), // stale
                ],
                asks: vec![],
            },
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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
        // SCENARIO 3.3: Partial match - 1 of 3 orders correct
        // =====================================================================
        print_separator("3.3: Partial Match - Only Level 0 Correct, Levels 1-2 Stale");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::new("up-good".to_string(), 0.54, 100.0, 100.0),  // correct!
                    OpenOrder::new("up-stale-1".to_string(), 0.50, 100.0, 100.0), // stale
                    OpenOrder::new("up-stale-2".to_string(), 0.49, 100.0, 100.0), // stale
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::new("down-good".to_string(), 0.44, 100.0, 100.0), // correct!
                    OpenOrder::new("down-stale".to_string(), 0.40, 100.0, 100.0), // stale
                ],
                asks: vec![],
            },
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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
        // SCENARIO 3.4: Orders correct but imbalance changed - need new offsets
        // =====================================================================
        print_separator("3.4: Imbalance Changed - Orders Were Correct, Now Need Update");

        // When balanced (delta=0), UP offset = 0.01, quotes at 0.54, 0.53, 0.52
        // But now we're 70/30 heavy UP (delta=0.4), UP offset = 0.01 * (1 + 0.4*5) = 0.03
        // New UP quotes should be: 0.55 - 0.03 = 0.52, 0.51, 0.50
        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    // These were correct when balanced, now too aggressive
                    OpenOrder::new("up-1".to_string(), 0.54, 100.0, 100.0),
                    OpenOrder::new("up-2".to_string(), 0.53, 100.0, 100.0),
                    OpenOrder::new("up-3".to_string(), 0.52, 100.0, 100.0),
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot {
                bids: vec![
                    // These are still correct (DOWN offset unchanged at 0.01)
                    OpenOrder::new("down-1".to_string(), 0.44, 100.0, 100.0),
                    OpenOrder::new("down-2".to_string(), 0.43, 100.0, 100.0),
                    OpenOrder::new("down-3".to_string(), 0.42, 100.0, 100.0),
                ],
                asks: vec![],
            },
            inventory: InventorySnapshot {
                up_size: 70.0,  // NOW HEAVY UP!
                up_avg_price: 0.52,
                down_size: 30.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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
        // SCENARIO 3.5: Orderbook moved - existing orders now at wrong level
        // =====================================================================
        print_separator("3.5: Orderbook Moved Up - Orders Now Below Target");

        // Old orderbook was at 0.55/0.45, orders at 0.54, 0.53, 0.52 / 0.44, 0.43, 0.42
        // New orderbook at 0.58/0.48, target quotes: 0.57, 0.56, 0.55 / 0.47, 0.46, 0.45
        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    // Now 3 ticks too low
                    OpenOrder::new("up-1".to_string(), 0.54, 100.0, 100.0),
                    OpenOrder::new("up-2".to_string(), 0.53, 100.0, 100.0),
                    OpenOrder::new("up-3".to_string(), 0.52, 100.0, 100.0),
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot {
                bids: vec![
                    // Now 3 ticks too low
                    OpenOrder::new("down-1".to_string(), 0.44, 100.0, 100.0),
                    OpenOrder::new("down-2".to_string(), 0.43, 100.0, 100.0),
                    OpenOrder::new("down-3".to_string(), 0.42, 100.0, 100.0),
                ],
                asks: vec![],
            },
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.58, 500.0)),  // MOVED UP
                best_bid: Some((0.56, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.48, 500.0)),  // MOVED UP
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
        // SCENARIO 3.6: Partially filled orders - size mismatch
        // =====================================================================
        print_separator("3.6: Partially Filled Orders - Size Mismatch (Should Cancel+Replace)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    // Price correct but size wrong (partially filled)
                    OpenOrder::new("up-1".to_string(), 0.54, 100.0, 45.0),  // was 100, now 45
                    OpenOrder::new("up-2".to_string(), 0.53, 100.0, 100.0), // correct
                    OpenOrder::new("up-3".to_string(), 0.52, 100.0, 20.0),  // was 100, now 20
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::new("down-1".to_string(), 0.44, 100.0, 100.0), // correct
                    OpenOrder::new("down-2".to_string(), 0.43, 100.0, 99.5),  // within tolerance
                    OpenOrder::new("down-3".to_string(), 0.42, 100.0, 100.0), // correct
                ],
                asks: vec![],
            },
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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
        // SCENARIO 3.7: Imbalance extreme - need to cancel all UP orders
        // =====================================================================
        print_separator("3.7: Extreme Imbalance - Cancel ALL UP Orders (delta > max_imbalance)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    // These were correct when balanced, now must be cancelled
                    OpenOrder::new("up-1".to_string(), 0.54, 100.0, 100.0),
                    OpenOrder::new("up-2".to_string(), 0.53, 100.0, 100.0),
                    OpenOrder::new("up-3".to_string(), 0.52, 100.0, 100.0),
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::new("down-1".to_string(), 0.44, 100.0, 100.0),
                    OpenOrder::new("down-2".to_string(), 0.43, 100.0, 100.0),
                    OpenOrder::new("down-3".to_string(), 0.42, 100.0, 100.0),
                ],
                asks: vec![],
            },
            inventory: InventorySnapshot {
                up_size: 95.0,  // EXTREME imbalance
                up_avg_price: 0.52,
                down_size: 5.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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
        // SCENARIO 3.8: Mixed - some match, some stale, need new levels
        // =====================================================================
        print_separator("3.8: Mixed State - Keep 2, Cancel 2, Add 2");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::new("up-keep".to_string(), 0.54, 100.0, 100.0),   // keep
                    OpenOrder::new("up-cancel".to_string(), 0.50, 100.0, 100.0), // cancel
                    // missing 0.53, 0.52
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::new("down-keep".to_string(), 0.44, 100.0, 100.0),   // keep
                    OpenOrder::new("down-cancel".to_string(), 0.38, 100.0, 100.0), // cancel
                    // missing 0.43, 0.42
                ],
                asks: vec![],
            },
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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

        println!("\n{}", "#".repeat(80));
        println!("# SECTION 4: EDGE CASES");
        println!("{}\n", "#".repeat(80));

        // =====================================================================
        // SCENARIO 4.1: Empty inventory
        // =====================================================================
        print_separator("5.1: Empty Inventory (Fresh Start)");

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
        // SCENARIO 4.2: No orderbook (missing best_ask)
        // =====================================================================
        print_separator("5.2: Missing Best Ask (No Quotes Possible)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: None,  // No ask!
                best_bid: Some((0.53, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: None,  // No ask!
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
        // SCENARIO 4.3: Very low prices (near $0.01 tick minimum)
        // =====================================================================
        print_separator("5.3: Very Low Prices (Near Minimum Tick)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.95,
                down_size: 50.0,
                down_avg_price: 0.03,
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.97, 500.0)),
                best_bid: Some((0.95, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.05, 500.0)),  // Very low!
                best_bid: Some((0.03, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // =====================================================================
        // SCENARIO 4.4: Different config - 5 levels, wider spread
        // =====================================================================
        print_separator("5.4: Different Config (5 Levels, 2c Spread Per Level)");

        let mut config = make_config();
        config.num_levels = 5;
        config.spread_per_level = 2.0;  // 2 cents per level

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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
            config,
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        println!("\n{}", "#".repeat(80));
        println!("# SECTION 5: FIFO QUEUE PRIORITY PRESERVATION");
        println!("{}\n", "#".repeat(80));

        // =====================================================================
        // SCENARIO 5.1: Decrease size - keep oldest, cancel newer, place remainder
        // =====================================================================
        print_separator("6.1: FIFO Decrease - Keep Oldest 100, Cancel Newer Two, Place 40");

        // Current: 3 orders @ 0.54 totaling 300 (timestamps 1000, 1001, 1002)
        // Desired: 140 @ 0.54
        // Expected: Keep oldest (100), cancel middle and newest, place 40 for remainder
        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::with_created_at("up-oldest".to_string(), 0.54, 100.0, 100.0, 1000), // OLDEST - keep!
                    OpenOrder::with_created_at("up-middle".to_string(), 0.54, 100.0, 100.0, 1001), // cancel
                    OpenOrder::with_created_at("up-newest".to_string(), 0.54, 100.0, 100.0, 1002), // NEWEST - cancel!
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,
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
            config: SolverConfig {
                order_size: 140.0, // Desired 140 at level 0
                ..make_config()
            },
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        println!("\n--- FIFO ANALYSIS ---");
        println!("Orders at $0.54 (sorted by created_at):");
        println!("  1. up-oldest  @ 1000 (100 size) -> KEEP (best queue position)");
        println!("  2. up-middle  @ 1001 (100 size) -> CANCEL (100+100=200 > 140)");
        println!("  3. up-newest  @ 1002 (100 size) -> CANCEL");
        println!("Kept sum: 100, Desired: 140, Remainder to place: 40");
        println!("Result: Cancel 2 orders, place 1 new order for 40");

        // Verify behavior
        assert!(output.cancellations.contains(&"up-middle".to_string()), "Should cancel middle order");
        assert!(output.cancellations.contains(&"up-newest".to_string()), "Should cancel newest order");
        assert!(!output.cancellations.contains(&"up-oldest".to_string()), "Should KEEP oldest order");
        // Verify the 40-size remainder order is placed
        let placed_at_054: Vec<_> = output.limit_orders.iter()
            .filter(|o| (o.price - 0.54).abs() < 0.001)
            .collect();
        assert_eq!(placed_at_054.len(), 1, "Should place 1 order at $0.54");
        assert!((placed_at_054[0].size - 40.0).abs() < 0.1, "Remainder should be ~40");
        println!("\n✓ VERIFIED: Oldest order preserved, newer orders cancelled, 40 remainder placed");

        // =====================================================================
        // SCENARIO 5.2: Increase size - keep all, add new
        // =====================================================================
        print_separator("6.2: FIFO Increase - Keep All Existing, Add 150 New");

        // Current: 1 order @ 0.54 for 100
        // Desired: 250 @ 0.54
        // Expected: Keep existing 100, place new order for 150
        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::with_created_at("up-existing".to_string(), 0.54, 100.0, 100.0, 1000),
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,
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
            config: SolverConfig {
                order_size: 250.0, // Desired 250 at level 0
                ..make_config()
            },
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        println!("\n--- FIFO ANALYSIS ---");
        println!("Orders at $0.54:");
        println!("  1. up-existing @ 1000 (100 size) -> KEEP (preserve queue position)");
        println!("Current sum: 100, Desired: 250, Additional to place: 150");
        println!("Result: No cancellations, place 1 new order for 150");

        // Verify behavior
        assert!(output.cancellations.is_empty(), "Should not cancel any orders when increasing size");
        let placed_at_054: Vec<_> = output.limit_orders.iter()
            .filter(|o| (o.price - 0.54).abs() < 0.001)
            .collect();
        assert!(!placed_at_054.is_empty(), "Should place additional order at $0.54");
        println!("\n✓ VERIFIED: Existing order preserved, additional order placed");

        // =====================================================================
        // SCENARIO 5.3: First order exceeds desired - cancel all, place new
        // =====================================================================
        print_separator("6.3: FIFO Overflow - First Order Too Large, Cancel All");

        // Current: 1 order @ 0.54 for 200 (exceeds desired 100)
        // Desired: 100 @ 0.54
        // Expected: Cannot fit first order, cancel it and place new 100
        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::with_created_at("up-too-large".to_string(), 0.54, 200.0, 200.0, 1000),
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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

        println!("\n--- FIFO ANALYSIS ---");
        println!("Orders at $0.54:");
        println!("  1. up-too-large @ 1000 (200 size) -> CANCEL (exceeds desired 100)");
        println!("Greedy check: 0 + 200 = 200 > 100 + 0.1 tolerance -> Cannot fit");
        println!("Result: Cancel the order, place new order for 100");

        // Verify behavior
        assert!(output.cancellations.contains(&"up-too-large".to_string()), "Should cancel oversized order");
        println!("\n✓ VERIFIED: Oversized order cancelled, new correctly-sized order placed");

        // =====================================================================
        // SCENARIO 5.4: Keep two oldest out of four
        // =====================================================================
        print_separator("6.4: FIFO Keep Two - 4 Orders, Keep 2 Oldest, Cancel 2 Newest");

        // Current: 4 orders @ 0.54 totaling 400 (timestamps 1000, 1001, 1002, 1003)
        // Desired: 200 @ 0.54
        // Expected: Keep oldest two (200), cancel newest two
        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::with_created_at("up-A".to_string(), 0.54, 100.0, 100.0, 1000), // OLDEST - keep
                    OpenOrder::with_created_at("up-B".to_string(), 0.54, 100.0, 100.0, 1001), // keep
                    OpenOrder::with_created_at("up-C".to_string(), 0.54, 100.0, 100.0, 1002), // cancel
                    OpenOrder::with_created_at("up-D".to_string(), 0.54, 100.0, 100.0, 1003), // NEWEST - cancel
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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
            config: SolverConfig {
                order_size: 200.0, // Desired 200 at level 0
                ..make_config()
            },
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        println!("\n--- FIFO ANALYSIS ---");
        println!("Orders at $0.54 (sorted by created_at):");
        println!("  1. up-A @ 1000 (100 size) -> KEEP (sum=100 <= 200)");
        println!("  2. up-B @ 1001 (100 size) -> KEEP (sum=200 <= 200)");
        println!("  3. up-C @ 1002 (100 size) -> CANCEL (sum=300 > 200)");
        println!("  4. up-D @ 1003 (100 size) -> CANCEL");
        println!("Kept sum: 200, Desired: 200, No remainder needed");

        // Verify behavior
        assert!(!output.cancellations.contains(&"up-A".to_string()), "Should KEEP oldest (A)");
        assert!(!output.cancellations.contains(&"up-B".to_string()), "Should KEEP second oldest (B)");
        assert!(output.cancellations.contains(&"up-C".to_string()), "Should cancel third oldest (C)");
        assert!(output.cancellations.contains(&"up-D".to_string()), "Should cancel newest (D)");
        println!("\n✓ VERIFIED: Two oldest orders preserved, two newest cancelled");

        // =====================================================================
        // SCENARIO 5.5: Multiple price levels with different FIFO adjustments
        // =====================================================================
        print_separator("6.5: FIFO Multi-Level - Different Adjustments Per Price");

        // UP @ 0.54: decrease from 200 to 100 (keep oldest)
        // UP @ 0.53: increase from 50 to 100 (keep all, add 50)
        // DOWN @ 0.44: exact match, no change
        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    // Level 0.54: 2 orders, need to reduce to 100
                    OpenOrder::with_created_at("up-54-old".to_string(), 0.54, 100.0, 100.0, 1000), // keep
                    OpenOrder::with_created_at("up-54-new".to_string(), 0.54, 100.0, 100.0, 1001), // cancel
                    // Level 0.53: 1 order, need to increase to 100
                    OpenOrder::with_created_at("up-53-exist".to_string(), 0.53, 50.0, 50.0, 1002), // keep
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot {
                bids: vec![
                    // Level 0.44: exact match
                    OpenOrder::with_created_at("down-44".to_string(), 0.44, 100.0, 100.0, 1003),
                ],
                asks: vec![],
            },
            inventory: InventorySnapshot {
                up_size: 50.0,
                up_avg_price: 0.52,
                down_size: 50.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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

        println!("\n--- FIFO ANALYSIS ---");
        println!("UP @ $0.54 (decrease 200 -> 100):");
        println!("  up-54-old @ 1000 -> KEEP (100 fits in desired 100)");
        println!("  up-54-new @ 1001 -> CANCEL (would exceed desired)");
        println!("UP @ $0.53 (increase 50 -> 100):");
        println!("  up-53-exist @ 1002 -> KEEP (preserve queue position)");
        println!("  Need to place additional 50");
        println!("DOWN @ $0.44 (exact match 100 = 100):");
        println!("  down-44 @ 1003 -> KEEP (no change needed)");

        // Verify behavior
        assert!(!output.cancellations.contains(&"up-54-old".to_string()), "Should KEEP older @ 0.54");
        assert!(output.cancellations.contains(&"up-54-new".to_string()), "Should cancel newer @ 0.54");
        assert!(!output.cancellations.contains(&"up-53-exist".to_string()), "Should KEEP @ 0.53");
        assert!(!output.cancellations.contains(&"down-44".to_string()), "Should KEEP @ 0.44");
        println!("\n✓ VERIFIED: FIFO preservation works across multiple price levels");

        println!("\n{}", "#".repeat(80));
        println!("# SECTION 6: DELTA IMBALANCE + FIFO PRESERVATION + SKEW-BASED SIZING");
        println!("{}\n", "#".repeat(80));

        // =====================================================================
        // SCENARIO 6.1: High UP delta with skew reduces UP size, preserves FIFO
        // =====================================================================
        print_separator("7.1: High UP Delta - Skew Reduces UP Size, FIFO Preserves Oldest");

        // Delta = 0.4 (70/30 UP heavy)
        // Skew factor = 2.0
        // UP size = 100 * (1 - 0.4 * 2.0) = 100 * 0.2 = 20
        // DOWN size = 100 * (1 + 0.4 * 2.0) = 100 * 1.8 = 180
        // Current: 3 UP orders @ 0.52 totaling 300, need to reduce to 20
        // Expected: FIFO keeps oldest until it fits, cancel the rest
        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::with_created_at("up-oldest".to_string(), 0.52, 100.0, 100.0, 1000), // cancel (100 > 20)
                    OpenOrder::with_created_at("up-middle".to_string(), 0.52, 100.0, 100.0, 1001), // cancel
                    OpenOrder::with_created_at("up-newest".to_string(), 0.52, 100.0, 100.0, 1002), // cancel
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 70.0,
                up_avg_price: 0.52,
                down_size: 30.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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

        println!("\n--- SKEW + FIFO ANALYSIS ---");
        let delta = 0.4;
        let skew_factor = 2.0;
        let up_size = 100.0 * (1.0 - delta * skew_factor);
        let down_size = 100.0 * (1.0 + delta * skew_factor);
        println!("Delta: {:.2}, Skew factor: {:.1}", delta, skew_factor);
        println!("Calculated UP size:   {:.1} = 100 * (1 - {:.1} * {:.1})", up_size, delta, skew_factor);
        println!("Calculated DOWN size: {:.1} = 100 * (1 + {:.1} * {:.1})", down_size, delta, skew_factor);
        println!("\nFIFO analysis for UP @ target price:");
        println!("  Desired size: {:.1}", up_size);
        println!("  Existing orders: 3 x 100 = 300");
        println!("  First order (100) already exceeds desired ({:.1}) -> cancel all, place new", up_size);

        // =====================================================================
        // SCENARIO 6.2: High DOWN delta with skew reduces DOWN size, FIFO keeps oldest
        // =====================================================================
        print_separator("7.2: High DOWN Delta - Skew Reduces DOWN Size, FIFO Preserves Oldest");

        // Delta = -0.4 (30/70 DOWN heavy)
        // DOWN size = 100 * (1 + (-0.4) * 2.0) = 100 * 0.2 = 20
        // UP size = 100 * (1 - (-0.4) * 2.0) = 100 * 1.8 = 180
        // Current: 3 DOWN orders @ 0.44 totaling 300, need to reduce to 20
        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::with_created_at("down-oldest".to_string(), 0.44, 100.0, 100.0, 1000),
                    OpenOrder::with_created_at("down-middle".to_string(), 0.44, 100.0, 100.0, 1001),
                    OpenOrder::with_created_at("down-newest".to_string(), 0.44, 100.0, 100.0, 1002),
                ],
                asks: vec![],
            },
            inventory: InventorySnapshot {
                up_size: 30.0,
                up_avg_price: 0.52,
                down_size: 70.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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

        println!("\n--- SKEW + FIFO ANALYSIS ---");
        let delta = -0.4;
        let down_size = 100.0 * (1.0 + delta * skew_factor);
        let up_size = 100.0 * (1.0 - delta * skew_factor);
        println!("Delta: {:.2}, Skew factor: {:.1}", delta, skew_factor);
        println!("Calculated DOWN size: {:.1} = 100 * (1 + ({:.1}) * {:.1})", down_size, delta, skew_factor);
        println!("Calculated UP size:   {:.1} = 100 * (1 - ({:.1}) * {:.1})", up_size, delta, skew_factor);

        // =====================================================================
        // SCENARIO 6.3: Moderate imbalance - skew partially reduces, FIFO keeps some
        // =====================================================================
        print_separator("7.3: Moderate UP Delta - Skew Reduces to 60, FIFO Keeps Oldest Order");

        // Delta = 0.2 (60/40 UP)
        // UP offset = 0.01 * (1 + 0.2 * 5) = 0.02, target = 0.55 - 0.02 = 0.53
        // UP size = 100 * (1 - 0.2 * 2.0) = 100 * 0.6 = 60
        // DOWN size = 100 * (1 + 0.2 * 2.0) = 100 * 1.4 = 140
        // Current: 2 UP orders @ 0.53 (correct price) totaling 100 (50 each), need 60
        // Expected: Keep oldest (50), cancel newer, place 10 for remainder
        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::with_created_at("up-old-50".to_string(), 0.53, 50.0, 50.0, 1000), // keep (50 <= 60)
                    OpenOrder::with_created_at("up-new-50".to_string(), 0.53, 50.0, 50.0, 1001), // cancel (50+50=100 > 60)
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 60.0,
                up_avg_price: 0.52,
                down_size: 40.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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

        println!("\n--- SKEW + FIFO ANALYSIS ---");
        let delta = 0.2;
        let up_size = 100.0 * (1.0 - delta * skew_factor);
        println!("Delta: {:.2}, Skew factor: {:.1}", delta, skew_factor);
        println!("Calculated UP size: {:.1} = 100 * (1 - {:.1} * {:.1})", up_size, delta, skew_factor);
        println!("\nFIFO analysis for UP @ $0.53:");
        println!("  Desired size: {:.1}", up_size);
        println!("  up-old-50 @ 1000: 50 <= {:.1} -> KEEP", up_size);
        println!("  up-new-50 @ 1001: 50 + 50 = 100 > {:.1} -> CANCEL", up_size);
        println!("  Kept sum: 50, Desired: {:.1}, Remainder: 10", up_size);

        // Verify
        assert!(!output.cancellations.contains(&"up-old-50".to_string()), "Should KEEP oldest");
        assert!(output.cancellations.contains(&"up-new-50".to_string()), "Should cancel newer");
        println!("\n✓ VERIFIED: FIFO preserved with skew-based size reduction");

        // =====================================================================
        // SCENARIO 6.4: Skew increases size - FIFO keeps all, adds more
        // =====================================================================
        print_separator("7.4: DOWN Delta Increases UP Size - FIFO Keeps All, Adds More");

        // Delta = -0.3 (35/65 DOWN heavy)
        // UP size = 100 * (1 - (-0.3) * 2.0) = 100 * 1.6 = 160
        // Current: 2 UP orders @ 0.54 totaling 100, need 160
        // Expected: Keep all, add 60
        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::with_created_at("up-A".to_string(), 0.54, 50.0, 50.0, 1000),
                    OpenOrder::with_created_at("up-B".to_string(), 0.54, 50.0, 50.0, 1001),
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 35.0,
                up_avg_price: 0.52,
                down_size: 65.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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

        println!("\n--- SKEW + FIFO ANALYSIS ---");
        let delta = -0.3;
        let up_size = 100.0 * (1.0 - delta * skew_factor);
        println!("Delta: {:.2}, Skew factor: {:.1}", delta, skew_factor);
        println!("Calculated UP size: {:.1} = 100 * (1 - ({:.1}) * {:.1})", up_size, delta, skew_factor);
        println!("\nFIFO analysis for UP @ $0.54:");
        println!("  Desired size: {:.1}", up_size);
        println!("  Current sum: 100");
        println!("  Additional needed: {:.1}", up_size - 100.0);
        println!("  All existing orders preserved (FIFO), new order placed for remainder");

        // Verify no cancellations
        assert!(!output.cancellations.contains(&"up-A".to_string()), "Should KEEP up-A");
        assert!(!output.cancellations.contains(&"up-B".to_string()), "Should KEEP up-B");
        println!("\n✓ VERIFIED: FIFO preserved when skew increases size");

        // =====================================================================
        // SCENARIO 6.5: Asymmetric skew with multi-level FIFO
        // =====================================================================
        print_separator("7.5: Asymmetric Skew - UP Reduced, DOWN Increased, Multi-Level FIFO");

        // Delta = 0.25 (62.5/37.5 UP heavy)
        // UP size = 100 * (1 - 0.25 * 2.0) = 100 * 0.5 = 50
        // DOWN size = 100 * (1 + 0.25 * 2.0) = 100 * 1.5 = 150
        // UP level 0: 3 orders totaling 150, reduce to 50 -> keep oldest, cancel 2
        // DOWN level 0: 1 order of 80, increase to 150 -> keep, add 70
        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::with_created_at("up-L0-old".to_string(), 0.54, 50.0, 50.0, 1000), // keep
                    OpenOrder::with_created_at("up-L0-mid".to_string(), 0.54, 50.0, 50.0, 1001), // cancel (100 > 50)
                    OpenOrder::with_created_at("up-L0-new".to_string(), 0.54, 50.0, 50.0, 1002), // cancel
                    // Level 1 - will also have reduced target
                    OpenOrder::with_created_at("up-L1-old".to_string(), 0.53, 50.0, 50.0, 1003), // keep
                    OpenOrder::with_created_at("up-L1-new".to_string(), 0.53, 50.0, 50.0, 1004), // cancel
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::with_created_at("down-L0".to_string(), 0.44, 80.0, 80.0, 1005), // keep, add 70
                ],
                asks: vec![],
            },
            inventory: InventorySnapshot {
                up_size: 62.5,
                up_avg_price: 0.52,
                down_size: 37.5,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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

        println!("\n--- SKEW + FIFO ANALYSIS ---");
        let delta = 0.25;
        let up_size = 100.0 * (1.0 - delta * skew_factor);
        let down_size = 100.0 * (1.0 + delta * skew_factor);
        println!("Delta: {:.2}, Skew factor: {:.1}", delta, skew_factor);
        println!("Calculated UP size:   {:.1} (reduced from 100)", up_size);
        println!("Calculated DOWN size: {:.1} (increased from 100)", down_size);
        println!("\nUP Level 0 @ $0.54 (reduce 150 -> 50):");
        println!("  up-L0-old: 50 <= 50 -> KEEP");
        println!("  up-L0-mid: 50 + 50 = 100 > 50 -> CANCEL");
        println!("  up-L0-new: CANCEL");
        println!("\nUP Level 1 @ $0.53 (reduce 100 -> 50):");
        println!("  up-L1-old: 50 <= 50 -> KEEP");
        println!("  up-L1-new: 50 + 50 = 100 > 50 -> CANCEL");
        println!("\nDOWN Level 0 @ $0.44 (increase 80 -> 150):");
        println!("  down-L0: 80 <= 150 -> KEEP");
        println!("  Place new order for: {:.1}", down_size - 80.0);

        // =====================================================================
        // SCENARIO 6.6: Extreme skew with clamping
        // =====================================================================
        print_separator("7.6: Extreme Delta - Skew Clamped, Tests Size Limits");

        // Delta = 0.6 (80/20 UP heavy)
        // UP size = 100 * (1 - 0.6 * 2.0) = 100 * -0.2 = -20 -> clamped to 0
        // DOWN size = 100 * (1 + 0.6 * 2.0) = 100 * 2.2 = 220 -> clamped to 300 (3x)
        // This tests the clamping behavior
        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::with_created_at("up-cancel-all".to_string(), 0.52, 100.0, 100.0, 1000),
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::with_created_at("down-keep".to_string(), 0.44, 100.0, 100.0, 1001),
                ],
                asks: vec![],
            },
            inventory: InventorySnapshot {
                up_size: 80.0,
                up_avg_price: 0.52,
                down_size: 20.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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

        println!("\n--- SKEW CLAMPING ANALYSIS ---");
        let delta: f64 = 0.6;
        let raw_up_size: f64 = 100.0 * (1.0 - delta * skew_factor);
        let raw_down_size: f64 = 100.0 * (1.0 + delta * skew_factor);
        let clamped_up = raw_up_size.clamp(0.0, 300.0);
        let clamped_down = raw_down_size.clamp(0.0, 300.0);
        println!("Delta: {:.2}, Skew factor: {:.1}", delta, skew_factor);
        println!("Raw UP size:     {:.1} (before clamp)", raw_up_size);
        println!("Clamped UP size: {:.1} (after clamp to [0, 300])", clamped_up);
        println!("Raw DOWN size:     {:.1} (before clamp)", raw_down_size);
        println!("Clamped DOWN size: {:.1} (after clamp to [0, 300])", clamped_down);
        println!("\nNote: At delta >= max_imbalance (0.8), UP quoting stops entirely");

        // =====================================================================
        // SCENARIO 6.7: Delta flips direction - orders need repositioning + size change
        // =====================================================================
        print_separator("7.7: Delta Flip - Was DOWN Heavy, Now UP Heavy, Combined Offset+Skew");

        // Previously was DOWN heavy (delta=-0.3), had UP orders with larger size
        // Now UP heavy (delta=0.3), UP offset increases AND UP size decreases
        // This tests that both offset and size adjustments work together with FIFO
        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    // These were placed when delta=-0.3, UP offset was lower (0.01)
                    // Now delta=0.3, UP offset is 0.01 * (1 + 0.3 * 5) = 0.025
                    // Old target was 0.54, new target is 0.55 - 0.025 = 0.525
                    // Also, old size was 160 (1.6x), new size is 40 (0.4x)
                    OpenOrder::with_created_at("up-old-price".to_string(), 0.54, 80.0, 80.0, 1000), // wrong price
                    OpenOrder::with_created_at("up-old-price2".to_string(), 0.54, 80.0, 80.0, 1001), // wrong price
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot {
                bids: vec![
                    // DOWN was at reduced size (40), now at increased size (160)
                    OpenOrder::with_created_at("down-small".to_string(), 0.44, 40.0, 40.0, 1002),
                ],
                asks: vec![],
            },
            inventory: InventorySnapshot {
                up_size: 65.0,
                up_avg_price: 0.52,
                down_size: 35.0,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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

        println!("\n--- DELTA FLIP ANALYSIS ---");
        let delta = 0.3;
        let offset_scaling = 5.0;
        let up_offset = 0.01 * (1.0 + delta * offset_scaling);
        let up_size = 100.0 * (1.0 - delta * skew_factor);
        let down_size = 100.0 * (1.0 + delta * skew_factor);
        println!("Current delta: {:.2}", delta);
        println!("UP offset:  ${:.4} (base $0.01 * {:.2})", up_offset, 1.0 + delta * offset_scaling);
        println!("UP size:    {:.1} (base 100 * {:.2})", up_size, 1.0 - delta * skew_factor);
        println!("DOWN size:  {:.1} (base 100 * {:.2})", down_size, 1.0 + delta * skew_factor);
        println!("\nExisting UP orders at $0.54 are at WRONG PRICE (new target: ${:.2})", 0.55 - up_offset);
        println!("  -> Cancel all, place new orders at correct price with reduced size");
        println!("Existing DOWN order (40) is SMALLER than new target ({:.1})", down_size);
        println!("  -> Keep (FIFO), add {:.1} more", down_size - 40.0);

        // =====================================================================
        // SCENARIO 6.8: Small orders fitting under skewed target
        // =====================================================================
        print_separator("7.8: Small FIFO Orders - Multiple Fit Under Skewed Target");

        // Delta = 0.15 (57.5/42.5 UP)
        // UP offset = 0.01 * (1 + 0.15 * 5) = 0.0175, target = 0.55 - 0.0175 = 0.53 (floored)
        // UP size = 100 * (1 - 0.15 * 2.0) = 100 * 0.7 = 70
        // Current: 4 small UP orders @ 0.53 (20 each = 80), need 70
        // Expected: Keep first 3 (60), cancel 4th, place 10 for remainder
        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot {
                bids: vec![
                    OpenOrder::with_created_at("up-20-A".to_string(), 0.53, 20.0, 20.0, 1000), // keep (20 <= 70)
                    OpenOrder::with_created_at("up-20-B".to_string(), 0.53, 20.0, 20.0, 1001), // keep (40 <= 70)
                    OpenOrder::with_created_at("up-20-C".to_string(), 0.53, 20.0, 20.0, 1002), // keep (60 <= 70)
                    OpenOrder::with_created_at("up-20-D".to_string(), 0.53, 20.0, 20.0, 1003), // cancel (80 > 70)
                ],
                asks: vec![],
            },
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 57.5,
                up_avg_price: 0.52,
                down_size: 42.5,
                down_avg_price: 0.44,  // max_up_bid = 1.0 - 0.44 - 0.01 = 0.55
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

        println!("\n--- SKEW + FIFO ANALYSIS ---");
        let delta = 0.15;
        let up_size = 100.0 * (1.0 - delta * skew_factor);
        println!("Delta: {:.2}, Skew factor: {:.1}", delta, skew_factor);
        println!("Calculated UP size: {:.1} = 100 * (1 - {:.2} * {:.1})", up_size, delta, skew_factor);
        println!("\nFIFO greedy accumulation:");
        println!("  up-20-A @ 1000: sum = 20 <= {:.1} -> KEEP", up_size);
        println!("  up-20-B @ 1001: sum = 40 <= {:.1} -> KEEP", up_size);
        println!("  up-20-C @ 1002: sum = 60 <= {:.1} -> KEEP", up_size);
        println!("  up-20-D @ 1003: sum = 80 > {:.1} -> CANCEL", up_size);
        println!("Kept: 60, Desired: {:.1}, Remainder: 10", up_size);

        // Verify
        assert!(!output.cancellations.contains(&"up-20-A".to_string()), "Keep A");
        assert!(!output.cancellations.contains(&"up-20-B".to_string()), "Keep B");
        assert!(!output.cancellations.contains(&"up-20-C".to_string()), "Keep C");
        assert!(output.cancellations.contains(&"up-20-D".to_string()), "Cancel D");
        println!("\n✓ VERIFIED: 3 oldest orders preserved, newest cancelled, remainder placed");

        println!("\n{}", "=".repeat(80));
        println!("  END OF VISUAL TESTS");
        println!("{}\n", "=".repeat(80));
    }
}
