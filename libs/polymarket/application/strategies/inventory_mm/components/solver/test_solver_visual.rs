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
        println!("  min_profit_margin: ${:.2}", input.config.min_profit_margin);
        println!("  max_imbalance: {:.0}%", input.config.max_imbalance * 100.0);
        println!("  order_size: {}", input.config.order_size);
        println!("  offset_scaling: {:.1}", input.config.offset_scaling);
    }

    fn print_output(input: &SolverInput, output: &crate::application::strategies::inventory_mm::types::SolverOutput) {
        println!("\n--- OUTPUT ---");

        let delta = input.inventory.imbalance();

        println!("\nOffset Calculation (based on delta={:.4}, scaling={:.1}):", delta, input.config.offset_scaling);
        let up_offset = input.config.base_offset * (1.0 + delta.max(0.0) * input.config.offset_scaling);
        let down_offset = input.config.base_offset * (1.0 + (-delta).max(0.0) * input.config.offset_scaling);
        println!("  UP offset:   ${:.4} (base ${:.2} * {:.2})", up_offset, input.config.base_offset, 1.0 + delta.max(0.0) * input.config.offset_scaling);
        println!("  DOWN offset: ${:.4} (base ${:.2} * {:.2})", down_offset, input.config.base_offset, 1.0 + (-delta).max(0.0) * input.config.offset_scaling);

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

        if !output.taker_orders.is_empty() {
            println!("\nTaker Orders (immediate execution):");
            for t in &output.taker_orders {
                println!("  {:?} {} @ ${:.2} x {} (score: {:.2})", t.side, t.token_id, t.price, t.size, t.score);
            }
        }

        println!("\n--- SUMMARY ---");
        println!("  {} cancellations, {} new limit orders, {} takers",
            output.cancellations.len(),
            output.limit_orders.len(),
            output.taker_orders.len()
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
            min_profit_margin: 0.01,
            max_imbalance: 0.8,
            order_size: 100.0,
            spread_per_level: 1.0,
            offset_scaling: 5.0,
            skew_factor: 1.0,
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
                down_avg_price: 0.46,
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
                down_avg_price: 0.46,
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
                down_avg_price: 0.46,
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
                down_avg_price: 0.46,
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

        println!("\n{}", "#".repeat(80));
        println!("# SECTION 4: TAKER OPPORTUNITIES");
        println!("{}\n", "#".repeat(80));

        // =====================================================================
        // SCENARIO 4.1: Heavy DOWN with cheap UP ask - should take
        // =====================================================================
        print_separator("4.1: Heavy DOWN + Cheap UP Ask (Should TAKE UP)");

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
                best_ask: Some((0.50, 150.0)),  // Cheap UP!
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
        let delta = input.inventory.imbalance();
        println!("Need UP (delta={:.2}), checking UP ask at $0.50", delta);
        let old_cost = 20.0 * 0.52;
        let new_cost = 100.0 * 0.50;
        let new_up_avg = (old_cost + new_cost) / 120.0;
        let combined = new_up_avg + 0.46;
        println!("  old_up_cost = 20 * $0.52 = ${:.2}", old_cost);
        println!("  new_up_cost = 100 * $0.50 = ${:.2}", new_cost);
        println!("  new_up_avg = ${:.2} / 120 = ${:.4}", old_cost + new_cost, new_up_avg);
        println!("  combined = ${:.4} + $0.46 = ${:.4}", new_up_avg, combined);
        println!("  threshold = $0.99 (1 - min_profit_margin)");
        println!("  {} < $0.99? {} -> {}",
            combined,
            combined < 0.99,
            if combined < 0.99 { "TAKE!" } else { "NO TAKE" }
        );

        // =====================================================================
        // SCENARIO 4.2: Heavy UP with cheap DOWN ask - should take
        // =====================================================================
        print_separator("4.2: Heavy UP + Cheap DOWN Ask (Should TAKE DOWN)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 80.0,
                up_avg_price: 0.52,
                down_size: 20.0,
                down_avg_price: 0.45,
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.55, 500.0)),
                best_bid: Some((0.53, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.44, 150.0)),  // Cheap DOWN!
                best_bid: Some((0.42, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // =====================================================================
        // SCENARIO 4.3: Best ask is ours - should NOT take
        // =====================================================================
        print_separator("4.3: Best Ask is OURS - Should NOT Take (Would Self-Trade)");

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
                best_ask: Some((0.50, 150.0)),  // Would be good to take...
                best_bid: Some((0.48, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: true,  // ...but it's OURS!
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

        // =====================================================================
        // SCENARIO 4.4: Would be unprofitable - should NOT take
        // =====================================================================
        print_separator("4.4: Unprofitable Take - Combined Cost > $0.99 (No Take)");

        let input = SolverInput {
            up_token_id: "up_token".to_string(),
            down_token_id: "down_token".to_string(),
            up_orders: OrderSnapshot::default(),
            down_orders: OrderSnapshot::default(),
            inventory: InventorySnapshot {
                up_size: 80.0,
                up_avg_price: 0.55,  // High UP avg
                down_size: 20.0,
                down_avg_price: 0.44,
            },
            up_orderbook: OrderbookSnapshot {
                best_ask: Some((0.55, 500.0)),
                best_bid: Some((0.53, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            down_orderbook: OrderbookSnapshot {
                best_ask: Some((0.46, 150.0)),  // Combined would be 0.55 + ~0.45 = 1.00+
                best_bid: Some((0.44, 200.0)),
                best_bid_is_ours: false,
                best_ask_is_ours: false,
            },
            config: make_config(),
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        // Explain why no take
        println!("\n--- TAKER CALCULATION ---");
        println!("Need DOWN (delta=0.60), checking DOWN ask at $0.46");
        let old_down_cost = 20.0 * 0.44;
        let new_down_cost = 100.0 * 0.46;
        let new_down_avg = (old_down_cost + new_down_cost) / 120.0;
        let combined = 0.55 + new_down_avg;
        println!("  old_down_cost = 20 * $0.44 = ${:.2}", old_down_cost);
        println!("  new_down_cost = 100 * $0.46 = ${:.2}", new_down_cost);
        println!("  new_down_avg = ${:.2} / 120 = ${:.4}", old_down_cost + new_down_cost, new_down_avg);
        println!("  combined = $0.55 + ${:.4} = ${:.4}", new_down_avg, combined);
        println!("  threshold = $0.99");
        println!("  {} < $0.99? {} -> {}",
            combined,
            combined < 0.99,
            if combined < 0.99 { "TAKE!" } else { "NO TAKE" }
        );

        println!("\n{}", "#".repeat(80));
        println!("# SECTION 5: EDGE CASES");
        println!("{}\n", "#".repeat(80));

        // =====================================================================
        // SCENARIO 5.1: Empty inventory
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
        // SCENARIO 5.2: No orderbook (missing best_ask)
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
                down_avg_price: 0.46,
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
        // SCENARIO 5.3: Very low prices (near $0.01 tick minimum)
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
        // SCENARIO 5.4: Different config - 5 levels, wider spread
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
            config,
        };

        print_input(&input);
        let output = solve(&input);
        print_output(&input, &output);

        println!("\n{}", "=".repeat(80));
        println!("  END OF VISUAL TESTS");
        println!("{}\n", "=".repeat(80));
    }
}
