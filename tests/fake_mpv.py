"""Protocol simulator used only by subprocess integration tests. Never outputs audio."""

import asyncio
import json
import sys


async def main():
    socket = next(arg.split("=", 1)[1] for arg in sys.argv if arg.startswith("--input-ipc-server="))
    shutdown = asyncio.Event()
    properties = {"pause": False, "volume": 60, "duration": 0, "time-pos": 0}
    entry = 0

    async def client(reader, writer):
        nonlocal entry
        observers = {}

        async def send(message):
            writer.write((json.dumps(message) + "\n").encode())
            await writer.drain()

        async def notify(name, value):
            if name in observers:
                await send(
                    {"event": "property-change", "name": name, "data": value, "id": observers[name]}
                )

        try:
            while line := await reader.readline():
                message = json.loads(line)
                args = message["command"]
                response = {"request_id": message["request_id"], "error": "success"}
                if args[0] == "observe_property":
                    observers[args[2]] = args[1]
                    await notify(args[2], properties[args[2]])
                elif args[0] == "set_property":
                    properties[args[1]] = args[2]
                    await notify(args[1], args[2])
                elif args[0] == "get_property":
                    response["data"] = properties.get(args[1])
                elif args[0] == "seek":
                    properties["time-pos"] = max(
                        0,
                        args[1]
                        if args[2].startswith("absolute")
                        else properties["time-pos"] + args[1],
                    )
                    await notify("time-pos", properties["time-pos"])
                await send(response)
                if args[0] == "loadfile":
                    entry += 1
                    await send({"event": "start-file", "playlist_entry_id": entry})
                    if args[1] == "fail":
                        await send(
                            {
                                "event": "end-file",
                                "reason": "error",
                                "error": "loading failed",
                                "playlist_entry_id": entry,
                            }
                        )
                    else:
                        properties["duration"] = 30
                        await notify("duration", 30)
                        await send({"event": "file-loaded"})
                elif args[0] == "stop":
                    await send({"event": "end-file", "reason": "stop", "playlist_entry_id": entry})
                elif args[0] == "quit":
                    shutdown.set()
                    break
        finally:
            writer.close()
            await writer.wait_closed()

    server = await asyncio.start_unix_server(client, socket)
    async with server:
        await shutdown.wait()


if __name__ == "__main__":
    asyncio.run(main())
