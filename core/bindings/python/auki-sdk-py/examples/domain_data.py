"""Dev-only round trip. Requires AUKI_EMAIL/PASSWORD/DOMAIN_ID/CLIENT_ID.

Uploads a generated 17 MiB file (or the optional input path), streams it back,
checks SHA-256, reads existing portals/poses, and deletes its temporary record.
"""
import asyncio
import hashlib
import os
from pathlib import Path
import sys
import tempfile
import uuid

from auki_sdk import AukiSession


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(65536), b""):
            result.update(block)
    return result.hexdigest()


async def round_trip(input_path, output_path):
    session = await AukiSession.login_dev(os.environ["AUKI_EMAIL"], os.environ["AUKI_PASSWORD"], client_id=os.environ["AUKI_CLIENT_ID"])
    domain_id = os.environ["AUKI_DOMAIN_ID"]
    name = f"sdk-374-python-{uuid.uuid4()}"
    data = session.data(domain_id)
    try:
        domains = session.domains()
        page = await domains.list(limit=50)
        portals = await domains.portals(domain_id)
        poses = await data.poses()
        if portals:
            await domains.portal(domain_id, portals[0]["id"])
            await domains.for_portal(portals[0]["short_id"])
        if poses:
            await data.pose(poses[0]["id"])
        print(f"Listed {len(page['domains'])} Domains, {len(portals)} portals and {len(poses)} poses")
        print(f"Temporary record: {name}")
        with input_path.open("rb") as source_file:
            async def source(maximum):
                return await asyncio.to_thread(source_file.read, maximum)
            saved = await data.write_stream(input_path.stat().st_size, source, name=name, data_type="sdk-test.file.v1")
        assert (await data.get(saved["id"]))["size"] == input_path.stat().st_size
        with output_path.open("wb") as destination:
            async def sink(chunk):
                await asyncio.to_thread(destination.write, chunk)
            downloaded = await data.read_to(saved["id"], sink, max_bytes=input_path.stat().st_size, max_chunk_bytes=65536)
        assert digest(input_path) == digest(output_path), "round-trip checksum mismatch"
        print(f"Streamed {downloaded} bytes; SHA-256 matched")
    finally:
        try:
            # Reconcile even if the upload completed but its response was lost.
            records = await data.list(name=name)
            errors = []
            for record in records:
                try:
                    await data.delete(record["id"])
                except Exception as error:
                    errors.append(error)
            if errors:
                raise errors[0]
            assert not await data.list(name=name), "temporary record remains"
            print("Temporary data deleted")
        finally:
            await data.close()
            await session.close()


def main():
    with tempfile.TemporaryDirectory(prefix="auki-domain-data-") as directory:
        directory = Path(directory)
        source = Path(sys.argv[1]) if len(sys.argv) > 1 else directory / "source.bin"
        if len(sys.argv) == 1:
            with source.open("wb") as stream:
                for _ in range(256):
                    stream.write(b"auki-domain-data\n" * 4096)
                stream.write(b"final short part\n")
        asyncio.run(round_trip(source, directory / "download.bin"))


if __name__ == "__main__":
    main()
