"""通过 CDP 直接截取 WebView2 页面，用于区分“页面没渲染”和“窗口没合成”。

用法:
    python tools/cdp_shot.py shots/cdp.png
"""

import base64
import json
import sys
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
    out = sys.argv[1] if len(sys.argv) > 1 else "shots/cdp.png"
    ws = websocket.create_connection(page_ws(), timeout=8, suppress_origin=True)
    mid = 0

    def call(method, params=None):
        nonlocal mid
        mid += 1
        ws.send(json.dumps({"id": mid, "method": method, "params": params or {}}))
        while True:
            msg = json.loads(ws.recv())
            if msg.get("id") == mid:
                return msg

    call("Page.enable")
    res = call("Page.captureScreenshot", {"format": "png", "captureBeyondViewport": False})
    data = res.get("result", {}).get("data")
    if not data:
        raise SystemExit(f"no screenshot: {res}")
    with open(out, "wb") as f:
        f.write(base64.b64decode(data))
    print("saved", out)
    ws.close()


if __name__ == "__main__":
    main()
