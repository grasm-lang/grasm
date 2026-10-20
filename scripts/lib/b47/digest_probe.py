import http.server, threading, subprocess, sys
CH = sys.argv[1:]
got = []
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        a = self.headers.get("Authorization")
        if a is None:
            self.send_response(401)
            self.send_header("WWW-Authenticate", self.server.challenge)
            self.send_header("Content-Length", "0")
            self.end_headers()
        else:
            got.append(a)
            self.send_response(200); self.send_header("Content-Length", "0"); self.end_headers()
srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
threading.Thread(target=srv.serve_forever, daemon=True).start()
for ch in CH:
    srv.challenge = ch
    got.clear()
    subprocess.run(["curl", "-s", "-o", "/dev/null", "--digest", "-u", "Mufasa:Circle Of Life", f"http://127.0.0.1:{srv.server_address[1]}/dir/index.html?x=1"])
    print(repr(ch), "->", got[0] if got else None)
srv.shutdown()
