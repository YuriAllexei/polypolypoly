"""Simulation module - real-data simulation for the quoter.

This module provides tools for backtesting the quoter against real
orderbook data and historical fills from Polymarket.
"""

from model_tuning.simulation.loaders import (
    load_fills_from_json,
    load_oracle_from_json,
    load_orderbooks_from_json,
    load_orderbooks_from_raw,
    load_simulation_data,
    load_simulation_data_from_raw,
)
from model_tuning.simulation.models import (
    MatchedFill,
    Orderbook,
    OrderbookHistoryEntry,
    OrderbookLevel,
    OrderbookSnapshot,
    OracleSnapshot,
    PositionState,
    RealFill,
)
from model_tuning.simulation.simulator import (
    RealDataSimulator,
    SimulationResult,
)
from model_tuning.simulation.visualize import generate_simulation_report

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
    "MatchedFill",
    # Simulator
    "RealDataSimulator",
    "SimulationResult",
    # Visualization
    "generate_simulation_report",
]
