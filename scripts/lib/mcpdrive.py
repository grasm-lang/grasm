import asyncio, os, sys
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

SRV = sys.argv[1]

async def main():
    params = StdioServerParameters(command=SRV, args=[], env=os.environ.copy())
    async with stdio_client(params) as (r, w):
        async with ClientSession(r, w) as s:
            init = await s.initialize()
            print(f"HANDSHAKE   {init.server_info.name} {init.server_info.version} proto={init.protocol_version}")
            print(f"CAPS        {init.capabilities}")
            tools = await s.list_tools()
            print(f"TOOLS       {[t.name for t in tools.tools]}")
            for t in tools.tools:
                print(f"  {t.name}: schema={t.input_schema}")
            r1 = await s.call_tool("add", {"a": 2, "b": 3})
            print(f"add(2,3)    {r1.content[0].text!r} isError={r1.is_error}")
            r2 = await s.call_tool("divide", {"a": 1, "b": 0})
            print(f"div by 0    {r2.content[0].text!r} isError={r2.is_error}")
            r3 = await s.call_tool("shout", {"text": "grasm wörks"})
            print(f"shout       {r3.content[0].text!r}")
            res = await s.list_resources()
            print(f"RESOURCES   {[str(x.uri) for x in res.resources]}")
            rr = await s.read_resource(res.resources[0].uri)
            print(f"read        {rr.contents[0].text!r}")
            pr = await s.list_prompts()
            print(f"PROMPTS     {[p.name for p in pr.prompts]}")
            gp = await s.get_prompt("review", {"language": "Grasm"})
            print(f"prompt      {gp.messages[0].content.text!r}")
            await s.send_ping()
            print("PING        ok")
            try:
                await s.call_tool("nosuch", {})
                print("BAD: missing tool was not an error")
            except Exception as e:
                print(f"missing     refused: {type(e).__name__}: {str(e)[:60]}")
            try:
                await s.get_prompt("review", {})
                print("BAD: missing required argument accepted")
            except Exception as e:
                print(f"missing arg refused: {str(e)[:60]}")
    print("ALL OK")

asyncio.run(main())
