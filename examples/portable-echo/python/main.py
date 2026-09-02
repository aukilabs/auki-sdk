from __future__ import annotations

import asyncio
import os
import sys

from auki_portable_echo import AukiEcho, AukiSession


async def main() -> None:
    target = parse_target(sys.argv[1:])

    session = await AukiSession.login_dev(
        os.environ["AUKI_EMAIL"],
        os.environ["AUKI_PASSWORD"],
    )
    peer = await session.start_peer(
        os.environ["AUKI_DOMAIN_ID"],
        os.environ.get("AUKI_IDENTITY_FILE", "./state/python-peer.identity"),
        discovery_mode=os.environ.get(
            "AUKI_DISCOVERY_MODE", "discover_and_advertise"
        ),
    )
    try:
        echo = await AukiEcho.mount(peer)
        try:
            routes = peer.routes
            print(f"peer: {peer.peer_id}")
            print(f"route: {routes.tcp}")
            print(f"wss route: {routes.wss}")

            if target and target[0] == "discovered":
                receipt = await send_discovered(echo, peer, target[1])
                print(f"echo: {receipt.payload.decode(errors='replace')}")
            elif target:
                print("using manual exact target fallback")
                receipt = await echo.send_exact(
                    target[1], target[2], b"hello from Auki"
                )
                print(f"echo: {receipt.payload.decode(errors='replace')}")
            else:
                try:
                    print_candidates(await peer.discover_protocol(echo.protocol))
                except Exception as error:
                    print(f"refresh Echo discovery failed: {error}")
                print(
                    "serving; use --discover PEER_ID from another terminal "
                    "or press Ctrl-C to stop"
                )
                await peer.wait_stopped()
        finally:
            await echo.close()
    finally:
        await peer.shutdown()


def parse_target(arguments: list[str]) -> tuple[str, str, str | None] | None:
    if not arguments:
        return None
    if len(arguments) == 2 and arguments[0] == "--discover":
        return ("discovered", arguments[1], None)
    if len(arguments) == 2:
        return ("manual", arguments[0], arguments[1])
    raise SystemExit(
        "usage: python main.py [--discover PEER_ID | PEER_ID EXACT_ROUTE]"
    )


def print_candidates(candidates: list[object]) -> None:
    print("discovered Echo peers (untrusted until exact dial):")
    if not candidates:
        print("  none")
    for candidate in candidates:
        print(
            f"  {candidate.peer_id} expires={candidate.expires_at} "
            f"routes={len(candidate.routes)}"
        )


def preferred_native_routes(routes: list[str]) -> list[str]:
    compatible = [route for route in routes if "/wss" not in route]
    return sorted(compatible, key=lambda route: "/p2p-circuit/" not in route)


async def send_discovered(echo: AukiEcho, peer: object, peer_id: str):
    candidates = await peer.discover_protocol(echo.protocol)
    print_candidates(candidates)
    candidate = next(
        (candidate for candidate in candidates if candidate.peer_id == peer_id),
        None,
    )
    if candidate is None:
        raise RuntimeError(f"Echo peer {peer_id} was not discovered")
    routes = preferred_native_routes(candidate.routes)
    if not routes:
        raise RuntimeError(
            f"Echo peer {peer_id} advertised no native-compatible route"
        )
    failures: list[str] = []
    for route in routes:
        try:
            return await echo.send_exact(peer_id, route, b"hello from Auki")
        except Exception as error:
            failures.append(f"{route}: {error}")
    raise RuntimeError(
        f"every discovered route for Echo peer {peer_id} failed: "
        + "; ".join(failures)
    )


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass
