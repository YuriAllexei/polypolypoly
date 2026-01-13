"""Pytest fixtures for model_tuning tests."""

import pytest

from model_tuning.core.models import Inventory, Market, Oracle
from model_tuning.core.quoter import InventoryMMQuoter, QuoterParams
from model_tuning.data.loaders import generate_synthetic_ticks
from model_tuning.tuning.backtester import Backtester, FillSimulator, MarketTick


@pytest.fixture
def default_params() -> QuoterParams:
    """Default quoter parameters."""
    return QuoterParams()


@pytest.fixture
def quoter(default_params: QuoterParams) -> InventoryMMQuoter:
    """Quoter with default parameters."""
    return InventoryMMQuoter(default_params)


@pytest.fixture
def balanced_inventory() -> Inventory:
    """Balanced inventory (q=0)."""
    return Inventory(
        up_qty=100,
        up_avg=0.48,
        down_qty=100,
        down_avg=0.48,
    )


@pytest.fixture
def overweight_up_inventory() -> Inventory:
    """Overweight UP inventory (q>0)."""
    return Inventory(
        up_qty=150,
        up_avg=0.55,
        down_qty=50,
        down_avg=0.45,
    )


@pytest.fixture
def neutral_market() -> Market:
    """50/50 market with 2c spread."""
    return Market(
        best_ask_up=0.51,
        best_bid_up=0.49,
        best_ask_down=0.51,
        best_bid_down=0.49,
    )


@pytest.fixture
def up_favored_market() -> Market:
    """Market favoring UP (55/45)."""
    return Market(
        best_ask_up=0.56,
        best_bid_up=0.54,
        best_ask_down=0.46,
        best_bid_down=0.44,
    )


@pytest.fixture
def neutral_oracle() -> Oracle:
    """Oracle exactly at threshold."""
    return Oracle(
        current_price=97000,
        threshold=97000,
    )


@pytest.fixture
def bullish_oracle() -> Oracle:
    """Oracle above threshold."""
    return Oracle(
        current_price=97200,
        threshold=97000,
    )


@pytest.fixture
def synthetic_ticks() -> list[MarketTick]:
    """5 minutes of synthetic tick data."""
    return generate_synthetic_ticks(
        duration_minutes=5.0,
        random_seed=42,
    )


@pytest.fixture
def backtester() -> Backtester:
    """Backtester with deterministic fills."""
    return Backtester(
        fill_simulator=FillSimulator(random_seed=42),
    )
