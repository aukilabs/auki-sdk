import asyncio
import os
import sys

from auki_portable_echo import AukiEcho, AukiSession


async def main() -> None:
    remote = sys.argv[1:]
    if len(remote) not in (0, 2):
        raise SystemExit("usage: python main.py [REMOTE_PEER_ID REMOTE_ROUTE]")

    session = await AukiSession.login_dev(
        os.environ["AUKI_EMAIL"],
        os.environ["AUKI_PASSWORD"],
    )
    peer = await session.start_peer(
        os.environ["AUKI_DOMAIN_ID"],
        os.environ.get("AUKI_IDENTITY_FILE", "./state/python-peer.identity"),
    )
    try:
        echo = await AukiEcho.mount(peer)
        try:
            routes = peer.routes
            print(f"peer: {peer.peer_id}")
            print(f"route: {routes.tcp}")
            print(f"wss route: {routes.wss}")

            if remote:
                receipt = await echo.send_exact(
                    remote[0], remote[1], b"hello from Auki"
                )
                print(f"echo: {receipt.payload.decode(errors='replace')}")
            else:
                print("serving; press Ctrl-C to stop")
                await peer.wait_stopped()
        finally:
            await echo.close()
    finally:
        await peer.shutdown()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass
