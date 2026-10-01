// Upstream backend shared by every proxy run.
//
// An HTTP/1.1 keep-alive server that answers every request with a fixed body
// (default 4096 bytes) and echoes, in response headers, what the proxy did to
// the request: the X-Forwarded-For it received and the length of the request
// body it received. The load generator checks both.
package main

import (
	"flag"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"strconv"
)

func main() {
	port := flag.Int("port", 9300, "listen port")
	size := flag.Int("size", 4096, "response body size in bytes")
	flag.Parse()

	body := make([]byte, *size)
	for i := range body {
		body[i] = 'a' + byte(i%26)
	}
	sizeStr := strconv.Itoa(*size)

	h := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		n, _ := io.Copy(io.Discard, r.Body)
		hd := w.Header()
		hd["X-Echo-Xff"] = []string{r.Header.Get("X-Forwarded-For")}
		hd["X-Echo-Xff-Count"] = []string{strconv.Itoa(len(r.Header.Values("X-Forwarded-For")))}
		hd["X-Echo-Len"] = []string{strconv.FormatInt(n, 10)}
		hd["Content-Type"] = []string{"application/octet-stream"}
		hd["Content-Length"] = []string{sizeStr}
		w.WriteHeader(200)
		w.Write(body)
	})

	ln, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", *port))
	if err != nil {
		log.Fatal(err)
	}
	srv := &http.Server{Handler: h}
	log.Fatal(srv.Serve(ln))
}
