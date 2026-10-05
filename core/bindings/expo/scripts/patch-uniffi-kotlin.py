#!/usr/bin/env python3
"""Make UniFFI 0.31 Kotlin compile with the Kotlin version Expo uses.

Two generator outputs are rejected:

- Objects with an async `close()` also implement `AutoCloseable.close()`.
  Kotlin treats those as conflicting overloads. Drop `AutoCloseable` and the
  synchronous closer on those types. The cleaner still frees the handle.
- Error variants with a field named `message` collide with `Throwable.message`.
  Rename that field to `errorMessage`.
"""

from __future__ import annotations

import sys
from pathlib import Path

SYNC_CLOSE = """    @Synchronized
    override fun close() {
        this.destroy()
    }

"""


def patch(text: str) -> str:
    message_fields = text.count("val `message`: kotlin.String")
    if message_fields == 0:
        raise SystemExit("UniFFI Kotlin has no `message` error fields to rename")
    text = text.replace("val `message`: kotlin.String", "val errorMessage: kotlin.String")
    text = text.replace("${ `message` }", "${ errorMessage }")
    text = text.replace("value.`message`", "value.errorMessage")

    parts = text.split("open class ")
    patched_closes = 0
    rebuilt = [parts[0]]
    for part in parts[1:]:
        if "override suspend fun `close`()" in part:
            if ", AutoCloseable," not in part.split("\n", 1)[0]:
                raise SystemExit("async close class is missing AutoCloseable")
            if SYNC_CLOSE not in part:
                raise SystemExit("async close class is missing the synchronous closer")
            header, _, body = part.partition("\n")
            header = header.replace(", AutoCloseable,", ",", 1)
            body = body.replace(SYNC_CLOSE, "", 1)
            part = header + "\n" + body
            patched_closes += 1
        rebuilt.append(part)
    if patched_closes == 0:
        raise SystemExit("no async close methods were patched")
    return "open class ".join(rebuilt)


def main() -> None:
    path = Path(sys.argv[1])
    path.write_text(patch(path.read_text()))


if __name__ == "__main__":
    main()
