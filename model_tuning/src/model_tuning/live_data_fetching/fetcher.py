"""WebSocket fetcher for live fill, oracle, and orderbook data from Polymarket.

Subscribes to orders_matched activity, crypto prices, and orderbook updates
for a given market slug and saves data to JSON files compatible with the simulator.
"""

import argparse
import asyncio
import json
import os
import signal
from datetime import datetime, timedelta
from pathlib import Path
from typing import Any

import aiohttp
import websockets
from dotenv import load_dotenv
from rich import print as rprint
from websockets.typing import Origin

# Load .env file from project root
load_dotenv(Path(__file__).parent.parent.parent.parent.parent / ".env")
load_dotenv()  # Also check current directory

LIVE_DATA_WS_URL = "wss://ws-live-data.polymarket.com/"
ORDERBOOK_WS_URL = "wss://ws-subscriptions-clob.polymarket.com/ws/market"
PING_INTERVAL_SECONDS = 8

# Chainlink Candlestick API
CHAINLINK_API_URL = "https://priceapi.dataengine.chain.link"


class DataFetcher:
    """Fetches live fill, oracle, and orderbook data from Polymarket WebSockets.

    Subscribes to orders_matched activity, crypto_prices_chainlink, and orderbook
    updates for a given market slug. Saves to sim_data/<slug>/.
    """

    def __init__(self, slug: str) -> None:
        """Initialize the fetcher.

        Args:
            slug: Market slug (e.g., "btc-updown-15m-1768511700")
        """
        self.slug = slug
        self.output_dir = Path("sim_data") / slug

        # Paths
        self.fills_path = self.output_dir / "fills.json"
        self.oracle_path = self.output_dir / "oracle.json"
        self.orderbook_raw_path = self.output_dir / "orderbooks_raw.json"

        # Data storage
        self.fills: list[dict[str, Any]] = []
        self.oracle: list[dict[str, Any]] = []
        self.initial_snapshots: dict[str, dict[str, Any]] = {}
        self.price_changes: list[dict[str, Any]] = []

        # Token IDs (fetched at start)
        self.up_token_id: str | None = None
        self.down_token_id: str | None = None

        # Threshold (fetched at start)
        self.threshold: float = 0.0

        # Control
        self._shutdown_event = asyncio.Event()

    @property
    def _running(self) -> bool:
        """Check if still running (for backwards compat)."""
        return not self._shutdown_event.is_set()

    async def _fetch_market_info(self) -> tuple[str, str, str]:
        """Fetch market info including token IDs and end date.

        Returns:
            Tuple of (up_token_id, down_token_id, end_date_iso)
        """
        url = f"https://gamma-api.polymarket.com/markets/slug/{self.slug}"
        async with aiohttp.ClientSession() as session:
            async with session.get(url) as response:
                data = await response.json()
                token_ids = json.loads(data["clobTokenIds"])
                end_date = data["endDate"]  # ISO format string
                # First is Up, second is Down (matches outcomes order)
                return token_ids[0], token_ids[1], end_date

    async def _get_chainlink_token(self, session: aiohttp.ClientSession) -> str:
        """Get JWT token from Chainlink API.

        Requires CHAINLINK_CLIENT_ID and CHAINLINK_CANDLESTICK_API_KEY env vars.

        Returns:
            JWT access token
        """
        client_id = os.environ.get("CHAINLINK_CLIENT_ID")
        api_key = os.environ.get("CHAINLINK_CANDLESTICK_API_KEY")

        if not client_id or not api_key:
            raise ValueError(
                "CHAINLINK_CLIENT_ID and CHAINLINK_CANDLESTICK_API_KEY must be set"
            )

        url = f"{CHAINLINK_API_URL}/api/v1/authorize"
        payload = {"login": client_id, "password": api_key}

        async with session.post(url, json=payload) as response:
            data = await response.json()
            if data.get("s") != "ok":
                raise ValueError(f"Chainlink auth failed: {data.get('errmsg', 'unknown')}")
            return data["d"]["access_token"]

    async def _fetch_threshold(self, end_date_iso: str) -> float:
        """Fetch the threshold (price to beat) from Chainlink Candlestick API.

        Uses the candle OPEN price at market start time (15 minutes before end).

        Args:
            end_date_iso: Market end date in ISO format (e.g., "2026-01-16T05:00:00Z")

        Returns:
            The open price (threshold) for this market
        """
        # Parse end_date and compute start_date (15 minutes before)
        end_dt = datetime.fromisoformat(end_date_iso.replace("Z", "+00:00"))
        start_dt = end_dt - timedelta(minutes=15)
        start_ts = int(start_dt.timestamp())

        # Extract symbol (e.g., "btc" -> "BTCUSD")
        symbol = self.slug.split("-")[0].upper() + "USD"

        timeout = aiohttp.ClientTimeout(total=15)

        try:
            async with aiohttp.ClientSession(timeout=timeout) as session:
                # Get JWT token
                token = await self._get_chainlink_token(session)

                # Fetch candles around the start timestamp
                from_ts = start_ts - 900  # 15 min before
                to_ts = start_ts + 900    # 15 min after

                url = (
                    f"{CHAINLINK_API_URL}/api/v1/history"
                    f"?symbol={symbol}&resolution=15m&from={from_ts}&to={to_ts}"
                )

                headers = {"Authorization": f"Bearer {token}"}

                async with session.get(url, headers=headers) as response:
                    data = await response.json()

                    if data.get("s") != "ok":
                        raise ValueError(f"Chainlink history failed: {data.get('errmsg')}")

                    timestamps = data.get("t", [])
                    opens = data.get("o", [])

                    if not timestamps or not opens:
                        raise ValueError("No candle data returned")

                    # Find the candle at or before start_ts
                    best_idx = 0
                    for i, ts in enumerate(timestamps):
                        if ts <= start_ts:
                            best_idx = i

                    # Chainlink returns prices in 18-decimal format
                    price = opens[best_idx] / 1e18

                    rprint(f"[green]Chainlink price at {start_ts}: ${price:,.2f}[/green]")
                    return price

        except Exception as e:
            rprint(f"[red]Chainlink API error: {e}[/red]")
            rprint("[yellow]Falling back to Polymarket API...[/yellow]")
            return await self._fetch_threshold_fallback(end_date_iso)

    async def _fetch_threshold_fallback(self, end_date_iso: str) -> float:
        """Fallback: fetch threshold from Polymarket API."""
        end_dt = datetime.fromisoformat(end_date_iso.replace("Z", "+00:00"))
        start_dt = end_dt - timedelta(minutes=15)

        event_start_time = start_dt.strftime("%Y-%m-%dT%H:%M:%SZ")
        end_date = end_dt.strftime("%Y-%m-%dT%H:%M:%SZ")
        symbol = self.slug.split("-")[0].upper()

        url = (
            f"https://polymarket.com/api/crypto/crypto-price"
            f"?eventStartTime={event_start_time}"
            f"&variant=fifteen"
            f"&endDate={end_date}"
            f"&symbol={symbol}"
        )

        headers = {
            "User-Agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36",
            "Accept": "application/json",
            "Referer": "https://polymarket.com/",
        }

        try:
            async with aiohttp.ClientSession() as session:
                async with session.get(url, headers=headers, timeout=aiohttp.ClientTimeout(total=10)) as response:
                    data = await response.json()
                    return float(data["openPrice"])
        except Exception as e:
            rprint(f"[red]Fallback API also failed: {e}[/red]")
            rprint("[yellow]Using 0 as threshold - will update from first oracle[/yellow]")
            return 0.0

    def _extract_symbol(self) -> str:
        """Extract crypto symbol from slug.

        "btc-updown-15m-1768511700" → "btc/usd"
        """
        parts = self.slug.split("-")
        return f"{parts[0]}/usd"

    def _build_live_data_subscribe_message(self) -> str:
        """Build the WebSocket subscription message for fills and oracle."""
        fills_filters = json.dumps({"event_slug": self.slug}, separators=(",", ":"))
        oracle_filters = json.dumps({"symbol": self._extract_symbol()}, separators=(",", ":"))
        message = {
            "action": "subscribe",
            "subscriptions": [
                {"topic": "activity", "type": "orders_matched", "filters": fills_filters},
                {"topic": "crypto_prices_chainlink", "type": "update", "filters": oracle_filters},
            ],
        }
        return json.dumps(message, separators=(",", ":"))

    def _build_orderbook_subscribe_message(self) -> str:
        """Build the WebSocket subscription message for orderbook."""
        message = {
            "assets_ids": [self.up_token_id, self.down_token_id],
            "type": "market",
        }
        return json.dumps(message, separators=(",", ":"))

    def _transform_fill_payload(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Transform WebSocket payload to simulator RealFill format."""
        timestamp = payload["timestamp"]
        if timestamp < 1e12:  # If in seconds, convert to ms
            timestamp = timestamp * 1000
        return {
            "price": payload["price"],
            "size": payload["size"],
            "side": payload["side"].lower(),
            "timestamp": timestamp,
            "outcome": payload["outcome"].lower(),
        }

    def _transform_oracle_payload(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Transform WebSocket payload to simulator OracleSnapshot format."""
        return {
            "price": payload["value"],
            "threshold": self.threshold,
            "timestamp": payload["timestamp"],
        }

    def _transform_initial_snapshot(self, snapshot: dict[str, Any]) -> dict[str, Any]:
        """Transform initial orderbook snapshot."""
        return {
            "timestamp": int(snapshot["timestamp"]),
            "bids": [
                {"price": float(level["price"]), "size": float(level["size"])}
                for level in snapshot.get("bids", [])
            ],
            "asks": [
                {"price": float(level["price"]), "size": float(level["size"])}
                for level in snapshot.get("asks", [])
            ],
        }

    def _transform_price_change(
        self, change: dict[str, Any], timestamp: int
    ) -> dict[str, Any]:
        """Transform a single price change."""
        return {
            "timestamp": timestamp,
            "asset_id": change["asset_id"],
            "price": float(change["price"]),
            "size": float(change["size"]),
            "side": change["side"].lower(),
        }

    def _save_fills(self) -> None:
        """Save fills to JSON file."""
        with open(self.fills_path, "w") as f:
            json.dump(self.fills, f, indent=2)

    def _save_oracle(self) -> None:
        """Save oracle data to JSON file."""
        with open(self.oracle_path, "w") as f:
            json.dump(self.oracle, f, indent=2)

    def _save_orderbook_raw(self) -> None:
        """Save orderbook raw data (initial + deltas) to JSON file."""
        data = {
            "up_token_id": self.up_token_id,
            "down_token_id": self.down_token_id,
            "initial_snapshots": self.initial_snapshots,
            "price_changes": self.price_changes,
        }
        with open(self.orderbook_raw_path, "w") as f:
            json.dump(data, f)

    async def _ping_loop(
        self, websocket: websockets.WebSocketClientProtocol, name: str
    ) -> None:
        """Send PING every 8 seconds to keep connection alive."""
        while not self._shutdown_event.is_set():
            try:
                # Use wait_for so we can check shutdown event frequently
                await asyncio.wait_for(
                    asyncio.sleep(PING_INTERVAL_SECONDS),
                    timeout=PING_INTERVAL_SECONDS
                )
                if self._shutdown_event.is_set():
                    break
                await websocket.send("PING")
            except asyncio.TimeoutError:
                continue  # Check shutdown event and loop
            except asyncio.CancelledError:
                break
            except Exception as e:
                rprint(f"[yellow]{name} ping error: {e}[/yellow]")
                break

    async def _connect_live_data(self) -> None:
        """Connect to live data WebSocket for fills and oracle."""
        rprint(f"[blue]Connecting to {LIVE_DATA_WS_URL}...[/blue]")

        ping_task: asyncio.Task[None] | None = None
        try:
            async with websockets.connect(
                LIVE_DATA_WS_URL,
                origin=Origin("https://polymarket.com"),
                user_agent_header="Mozilla/5.0",
                ping_interval=None,
            ) as websocket:
                rprint("[green]Live data connected![/green]")

                # Subscribe
                subscribe_msg = self._build_live_data_subscribe_message()
                await websocket.send(subscribe_msg)
                rprint(f"[blue]Subscribed to orders_matched for {self.slug}[/blue]")
                rprint(f"[blue]Subscribed to crypto_prices for {self._extract_symbol()}[/blue]")

                # Start ping task
                ping_task = asyncio.create_task(self._ping_loop(websocket, "LiveData"))

                # Process messages
                while not self._shutdown_event.is_set():
                    try:
                        # Short timeout so we can check shutdown event frequently
                        message = await asyncio.wait_for(
                            websocket.recv(),
                            timeout=1.0
                        )

                        if isinstance(message, str) and message in ("PONG", "pong"):
                            continue
                        if not message:
                            continue

                        try:
                            data = json.loads(message)
                        except json.JSONDecodeError:
                            continue

                        msg_type = data.get("type")
                        topic = data.get("topic")
                        payload = data.get("payload")

                        if not payload:
                            continue

                        if msg_type == "orders_matched":
                            fill = self._transform_fill_payload(payload)
                            self.fills.append(fill)
                            rprint(
                                f"[green]Fill:[/green] {fill['outcome'].upper()} "
                                f"{fill['size']} @ {fill['price']:.3f} ({fill['side']})"
                            )
                            self._save_fills()

                        elif msg_type == "update" and topic == "crypto_prices_chainlink":
                            oracle_data = self._transform_oracle_payload(payload)
                            self.oracle.append(oracle_data)
                            rprint(
                                f"[cyan]Oracle:[/cyan] ${oracle_data['price']:,.2f} "
                                f"@ {oracle_data['timestamp']:.0f}"
                            )
                            self._save_oracle()

                    except asyncio.TimeoutError:
                        continue  # Check shutdown event and loop
                    except asyncio.CancelledError:
                        rprint("[yellow]Live data task cancelled[/yellow]")
                        break
                    except websockets.ConnectionClosed:
                        rprint("[yellow]Live data connection closed[/yellow]")
                        break

                # Graceful close
                await websocket.close()

        except asyncio.CancelledError:
            rprint("[yellow]Live data task cancelled[/yellow]")
        except Exception as e:
            rprint(f"[red]Live data error: {e}[/red]")

        finally:
            if ping_task:
                ping_task.cancel()
                try:
                    await ping_task
                except asyncio.CancelledError:
                    pass

    async def _connect_orderbook(self) -> None:
        """Connect to orderbook WebSocket for order book updates."""
        rprint(f"[blue]Connecting to {ORDERBOOK_WS_URL}...[/blue]")

        ping_task: asyncio.Task[None] | None = None
        try:
            async with websockets.connect(
                ORDERBOOK_WS_URL,
                origin=Origin("https://polymarket.com"),
                user_agent_header="Mozilla/5.0",
                ping_interval=None,
            ) as websocket:
                rprint("[green]Orderbook connected![/green]")

                # Subscribe
                subscribe_msg = self._build_orderbook_subscribe_message()
                await websocket.send(subscribe_msg)
                rprint("[blue]Subscribed to orderbook updates[/blue]")

                # Start ping task
                ping_task = asyncio.create_task(self._ping_loop(websocket, "Orderbook"))

                # Process messages
                while not self._shutdown_event.is_set():
                    try:
                        # Short timeout so we can check shutdown event frequently
                        message = await asyncio.wait_for(
                            websocket.recv(),
                            timeout=1.0
                        )

                        if isinstance(message, str) and message in ("PONG", "pong"):
                            continue
                        if not message:
                            continue

                        try:
                            data = json.loads(message)
                        except json.JSONDecodeError:
                            continue

                        # Initial snapshot is a list of 2 books
                        if isinstance(data, list):
                            for snapshot in data:
                                if snapshot.get("event_type") == "book":
                                    asset_id = snapshot["asset_id"]
                                    self.initial_snapshots[asset_id] = (
                                        self._transform_initial_snapshot(snapshot)
                                    )
                                    side = "UP" if asset_id == self.up_token_id else "DOWN"
                                    rprint(f"[magenta]Initial {side} orderbook received[/magenta]")
                            self._save_orderbook_raw()
                            continue

                        # Price change updates
                        event_type = data.get("event_type")
                        if event_type == "price_change":
                            timestamp = int(data["timestamp"])
                            for change in data.get("price_changes", []):
                                transformed = self._transform_price_change(change, timestamp)
                                self.price_changes.append(transformed)
                            # Save periodically (every 100 changes)
                            if len(self.price_changes) % 100 == 0:
                                self._save_orderbook_raw()
                                rprint(
                                    f"[dim]Orderbook: {len(self.price_changes)} price changes[/dim]"
                                )

                    except asyncio.TimeoutError:
                        continue  # Check shutdown event and loop
                    except asyncio.CancelledError:
                        rprint("[yellow]Orderbook task cancelled[/yellow]")
                        break
                    except websockets.ConnectionClosed:
                        rprint("[yellow]Orderbook connection closed[/yellow]")
                        break

                # Graceful close
                await websocket.close()

        except asyncio.CancelledError:
            rprint("[yellow]Orderbook task cancelled[/yellow]")
        except Exception as e:
            rprint(f"[red]Orderbook error: {e}[/red]")

        finally:
            if ping_task:
                ping_task.cancel()
                try:
                    await ping_task
                except asyncio.CancelledError:
                    pass

    async def _schedule_auto_stop(self, end_date_iso: str) -> None:
        """Schedule automatic stop 15 seconds before market ends.

        Args:
            end_date_iso: Market end date in ISO format
        """
        # Parse end date
        end_dt = datetime.fromisoformat(end_date_iso.replace("Z", "+00:00"))

        # Calculate seconds until 15 seconds before market end
        now = datetime.now(end_dt.tzinfo)
        stop_dt = end_dt - timedelta(seconds=15)
        seconds_until_stop = (stop_dt - now).total_seconds()

        if seconds_until_stop <= 0:
            rprint("[yellow]Market ending soon, stopping immediately[/yellow]")
            self.stop()
            return

        rprint(f"[blue]Will stop in {seconds_until_stop:.0f}s (15s before market end)[/blue]")

        try:
            # Sleep until stop time
            await asyncio.sleep(seconds_until_stop)
        except asyncio.CancelledError:
            rprint("[yellow]Auto-stop cancelled[/yellow]")
            return

        rprint("[yellow]Market ending soon, stopping...[/yellow]")
        self.stop()

    async def _connect_live_data_with_retry(self) -> None:
        """Connect to live data WebSocket with automatic reconnection."""
        retry_delay = 1
        max_retry_delay = 30

        while not self._shutdown_event.is_set():
            try:
                await self._connect_live_data()
            except asyncio.CancelledError:
                break
            except Exception as e:
                if self._shutdown_event.is_set():
                    break
                rprint(f"[yellow]Live data disconnected: {e}. Reconnecting in {retry_delay}s...[/yellow]")
                await asyncio.sleep(retry_delay)
                retry_delay = min(retry_delay * 2, max_retry_delay)
            else:
                # Connection closed normally, reset delay
                retry_delay = 1
                if not self._shutdown_event.is_set():
                    rprint("[yellow]Live data connection closed. Reconnecting...[/yellow]")

    async def _connect_orderbook_with_retry(self) -> None:
        """Connect to orderbook WebSocket with automatic reconnection."""
        retry_delay = 1
        max_retry_delay = 30

        while not self._shutdown_event.is_set():
            try:
                await self._connect_orderbook()
            except asyncio.CancelledError:
                break
            except Exception as e:
                if self._shutdown_event.is_set():
                    break
                rprint(f"[yellow]Orderbook disconnected: {e}. Reconnecting in {retry_delay}s...[/yellow]")
                await asyncio.sleep(retry_delay)
                retry_delay = min(retry_delay * 2, max_retry_delay)
            else:
                # Connection closed normally, reset delay
                retry_delay = 1
                if not self._shutdown_event.is_set():
                    rprint("[yellow]Orderbook connection closed. Reconnecting...[/yellow]")

    async def connect(self) -> None:
        """Connect to all WebSockets and stream data.

        Runs live data and orderbook connections concurrently with auto-reconnect.
        Only stops when auto-stop triggers or user presses Ctrl+C.
        """
        # Create output directory
        self.output_dir.mkdir(parents=True, exist_ok=True)

        # Fetch market info (token IDs + end date)
        rprint("[blue]Fetching market info...[/blue]")
        self.up_token_id, self.down_token_id, end_date = await self._fetch_market_info()
        rprint(f"[green]Up token: {self.up_token_id[:20]}...[/green]")
        rprint(f"[green]Down token: {self.down_token_id[:20]}...[/green]")

        # Fetch threshold (open price)
        rprint("[blue]Fetching threshold...[/blue]")
        self.threshold = await self._fetch_threshold(end_date)
        rprint(f"[green]Threshold: ${self.threshold:,.2f}[/green]")

        rprint(f"[blue]Saving to {self.output_dir}/[/blue]")

        # Create tasks with retry wrappers
        live_data_task = asyncio.create_task(self._connect_live_data_with_retry())
        orderbook_task = asyncio.create_task(self._connect_orderbook_with_retry())
        auto_stop_task = asyncio.create_task(self._schedule_auto_stop(end_date))

        tasks = [live_data_task, orderbook_task, auto_stop_task]

        try:
            # Wait for auto-stop task to complete (it's the only one that should end normally)
            await auto_stop_task
        except asyncio.CancelledError:
            rprint("[yellow]Received shutdown signal[/yellow]")
        finally:
            # Signal shutdown and cancel all tasks
            self.stop()
            for task in tasks:
                if not task.done():
                    task.cancel()
                    try:
                        await task
                    except asyncio.CancelledError:
                        pass

            # Final saves
            if self.fills:
                self._save_fills()
                rprint(f"[green]Final: {len(self.fills)} fills[/green]")
            if self.oracle:
                self._save_oracle()
                rprint(f"[green]Final: {len(self.oracle)} oracle snapshots[/green]")
            if self.price_changes or self.initial_snapshots:
                self._save_orderbook_raw()
                rprint(f"[green]Final: {len(self.price_changes)} orderbook changes[/green]")

    def stop(self) -> None:
        """Signal the fetcher to stop."""
        self._shutdown_event.set()


async def main(slug: str) -> None:
    """Main entry point for the fetcher.

    Args:
        slug: Market slug to subscribe to
    """
    fetcher = DataFetcher(slug)

    # Handle Ctrl+C gracefully
    loop = asyncio.get_event_loop()

    def signal_handler() -> None:
        rprint("\n[yellow]Shutting down...[/yellow]")
        fetcher.stop()

    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, signal_handler)

    await fetcher.connect()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(
        description="Fetch live fill, oracle, and orderbook data from Polymarket"
    )
    parser.add_argument("slug", help="Market slug (e.g., btc-updown-15m-1768511700)")

    args = parser.parse_args()

    rprint("[bold]Polymarket Data Fetcher[/bold]")
    rprint(f"Slug: {args.slug}")
    rprint(f"Output: sim_data/{args.slug}/")
    rprint("")

    asyncio.run(main(args.slug))
