"""通过 CDP 读取 WebView2 页面状态，诊断白屏/JS 错误。

用法:
    python tools/cdp.py "expression"
"""

import json
import sys
import time
import urllib.request

import websocket


def page_ws() -> str:
    with urllib.request.urlopen("http://127.0.0.1:9222/json", timeout=5) as r:
        pages = json.load(r)
    for p in pages:
        if p.get("type") == "page":
            return p["webSocketDebuggerUrl"]
    raise SystemExit("no page")


def main() -> None:
    expr = sys.argv[1] if len(sys.argv) > 1 else "document.title"
    ws = websocket.create_connection(page_ws(), timeout=5, suppress_origin=True)
    mid = 0

    def send(method, params=None):
        nonlocal mid
        mid += 1
        ws.send(json.dumps({"id": mid, "method": method, "params": params or {}}))
        while True:
            msg = json.loads(ws.recv())
            if msg.get("method") in ("Runtime.exceptionThrown", "Log.entryAdded"):
                print("EVENT", json.dumps(msg["params"], ensure_ascii=False)[:600])
            if msg.get("id") == mid:
                return msg

    send("Runtime.enable")
    send("Log.enable")
    res = send(
        "Runtime.evaluate",
        {"expression": expr, "returnByValue": True, "awaitPromise": True},
    )
    print(json.dumps(res.get("result", {}), ensure_ascii=False)[:2000])

    time.sleep(0.4)
    ws.settimeout(0.3)
    try:
        while True:
            msg = json.loads(ws.recv())
            if msg.get("method") in ("Runtime.exceptionThrown", "Log.entryAdded"):
                print("EVENT", json.dumps(msg["params"], ensure_ascii=False)[:800])
    except Exception:
        pass
    ws.close()


if __name__ == "__main__":
    main()
