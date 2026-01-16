"""Simulation module - real-data simulation for the quoter.

This module provides tools for backtesting the quoter against real
orderbook data and historical fills from Polymarket.
"""

from model_tuning.simulation.fill_driven_simulator import (
    FillDrivenSimulationResult,
    FillDrivenSimulator,
)
from model_tuning.simulation.loaders import (
    load_fills_from_json,
    load_oracle_from_json,
    load_orderbooks_from_json,
    load_orderbooks_from_raw,
    load_simulation_data,
    load_simulation_data_from_raw,
)
from model_tuning.simulation.models import (
    EnhancedPositionState,
    MatchedFill,
    Orderbook,
    OrderbookHistoryEntry,
    OrderbookLevel,
    OrderbookSnapshot,
    OracleSnapshot,
    PositionState,
    RealFill,
)
from model_tuning.simulation.orderbook_reconstructor import OrderbookReconstructor
from model_tuning.simulation.quoters import (
    BrainDeadQuoter,
    SimpleQuote,
    SimulationQuoter,
)
from model_tuning.simulation.simulator import (
    RealDataSimulator,
    SimulationResult,
)
from model_tuning.simulation.visualize import (
    generate_fill_driven_report,
    generate_simulation_report,
)

__all__ = [
    # Loaders
    "load_orderbooks_from_json",
    "load_orderbooks_from_raw",
    "load_fills_from_json",
    "load_oracle_from_json",
    "load_simulation_data",
    "load_simulation_data_from_raw",
    # Models
    "OrderbookSnapshot",
    "Orderbook",
    "OrderbookHistoryEntry",
    "OrderbookLevel",
    "RealFill",
    "OracleSnapshot",
    "PositionState",
    "EnhancedPositionState",
    "MatchedFill",
    # Orderbook Reconstructor
    "OrderbookReconstructor",
    # Quoters
    "BrainDeadQuoter",
    "SimpleQuote",
    "SimulationQuoter",
    # Original Simulator
    "RealDataSimulator",
    "SimulationResult",
    # Fill-Driven Simulator
    "FillDrivenSimulator",
    "FillDrivenSimulationResult",
    # Visualization
    "generate_simulation_report",
    "generate_fill_driven_report",
]
