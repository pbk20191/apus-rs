#!/bin/bash
# Drives the nw_mcp server with curl.
cd "$(dirname "$0")"
./target/debug/nw_mcp > server.out 2> server.err & SERVER=$!
trap 'kill $SERVER 2>/dev/null' EXIT
for _ in $(seq 50); do PORT=$(sed -n 's/.*127.0.0.1:\([0-9]*\).*/\1/p' server.out); [ -n "$PORT" ] && break; sleep 0.1; done
[ -n "$PORT" ] || { echo "server did not start"; cat server.err; exit 1; }
U=http://127.0.0.1:$PORT
post() { curl -s --max-time 10 -X POST $U/mcp -H 'Content-Type: application/json' -d "$1" "${@:2}"; }
text() { python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("result",{}).get("content",[{}])[0].get("text") or d.get("error") or d)'; }
echo "port $PORT; before any request: $(curl -s --max-time 5 $U/stats)"
echo "no session yet    → $(post '{"jsonrpc":"2.0","id":9,"method":"tools/list"}')"
echo "initialize        → $(post '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}' -D headers.out | python3 -c 'import json,sys; r=json.load(sys.stdin)["result"]; print(r["protocolVersion"], r["serverInfo"]["name"])') [$(grep -i mcp-session-id headers.out | tr -d '\r')]"
echo "initialized       → HTTP $(post '{"jsonrpc":"2.0","method":"notifications/initialized"}' -o /dev/null -w '%{http_code}')"
echo "tools/list        → $(post '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | python3 -c 'import json,sys; print([t["name"] for t in json.load(sys.stdin)["result"]["tools"]])')"
echo "sum               → $(post '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"sum","arguments":{"a":40,"b":2}}}' | text)"
echo "wait 120          → $(post '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"wait","arguments":{"millis":120}}}' | text)"
echo "heavy             → $(post '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"heavy","arguments":{}}}' | text)"
echo "unknown tool      → $(post '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"nope","arguments":{}}}' | text)"
echo "bad json          → HTTP $(post '{nope' -o /dev/null -w '%{http_code}')"
( post '{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"wait","arguments":{"millis":300}}}' | text | sed 's/^/concurrent slow   → /' ) & SLOW=$!
sleep 0.05
echo "concurrent fast   → $(post '{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"sum","arguments":{"a":1,"b":1}}}' | text)"
wait $SLOW
python3 - $U <<'PY'
import json, sys, time, http.client, urllib.parse
u = urllib.parse.urlparse(sys.argv[1]); c = http.client.HTTPConnection(u.hostname, u.port, timeout=5); samples = []
for i in range(200):
    body = json.dumps({"jsonrpc":"2.0","id":1000+i,"method":"tools/call","params":{"name":"sum","arguments":{"a":i,"b":1}}})
    t = time.perf_counter(); c.request("POST", "/mcp", body, {"Content-Type":"application/json"}); r = c.getresponse(); r.read(); samples.append((time.perf_counter()-t)*1e6)
samples.sort(); print(f"200 calls on one connection: median {samples[100]:.0f} µs, p95 {samples[190]:.0f} µs, max {samples[-1]:.0f} µs")
PY
A=$(curl -s --max-time 5 $U/stats); sleep 2; B=$(curl -s --max-time 5 $U/stats)
echo "stats             → $A; 2 s idle later: $B"
echo "DELETE            → HTTP $(curl -s --max-time 5 -X DELETE $U/mcp -o /dev/null -w '%{http_code}')"; sleep 0.3
A=$(curl -s --max-time 5 $U/stats); sleep 2; B=$(curl -s --max-time 5 $U/stats)
echo "after DELETE      → $A; 2 s idle later: $B"
echo "--- server stderr"; cat server.err
echo "--- threads of the server process"; sample $SERVER 1 2>/dev/null | grep -E "^    [0-9]+ Thread_|Thread_[0-9]+.*(DispatchQueue|tokio)" | sed 's/^ *//' | cut -c1-110
