// HTTP/1.1 reverse proxy subset: the Go entry of benchmarks/systems/proxy.
//
//	proxy [listen_port] [upstream_port]        (defaults 9301, 9300)
//
// Idiomatic Go over raw net.Conn (not httputil.ReverseProxy): one goroutine
// per client connection, each owning a keep-alive upstream connection. The
// request is parsed in place in the read buffer and forwarded with one
// net.Buffers (writev) of three sub-slices — head, the X-Forwarded-For bytes,
// the rest including the body — so it is not copied. Response chunks are
// written to the client straight out of the read buffer; the response head is
// scanned in place unless it straddles two reads.
package main

import (
	"bytes"
	"errors"
	"fmt"
	"log"
	"net"
	"os"
	"strconv"
)

type head struct {
	end    int // index of the CR of the blank line
	length int // Content-Length (0 when absent)
	xffEnd int // index of the CR ending an X-Forwarded-For line, or -1
}

var (
	crlfcrlf      = []byte("\r\n\r\n")
	nameLength    = []byte("content-length")
	nameForwarded = []byte("x-forwarded-for")
	errBad        = errors.New("unreadable head")
)

func nameIs(line, name []byte) bool {
	return len(line) > len(name) && line[len(name)] == ':' && bytes.EqualFold(line[:len(name)], name)
}

func number(b []byte) (int, bool) {
	i := 0
	for i < len(b) && (b[i] == ' ' || b[i] == '\t') {
		i++
	}
	v, any := 0, false
	for ; i < len(b) && b[i] >= '0' && b[i] <= '9'; i++ {
		v = v*10 + int(b[i]-'0')
		any = true
	}
	return v, any
}

// scanHead answers ok=false when the head is incomplete.
func scanHead(b []byte) (h head, ok bool, err error) {
	end := bytes.Index(b, crlfcrlf)
	if end < 0 {
		return h, false, nil
	}
	h = head{end: end, xffEnd: -1}
	ls := bytes.IndexByte(b, '\r') + 2
	for ls < end+2 {
		le := ls + bytes.IndexByte(b[ls:], '\r')
		line := b[ls:le]
		if nameIs(line, nameLength) {
			n, good := number(line[len(nameLength)+1:])
			if !good {
				return h, false, errBad
			}
			h.length = n
		} else if nameIs(line, nameForwarded) {
			h.xffEnd = le
		}
		ls = le + 2
	}
	return h, true, nil
}

func serve(client *net.TCPConn, upstreamAddr *net.TCPAddr) error {
	defer client.Close()
	upstream, err := net.DialTCP("tcp", nil, upstreamAddr)
	if err != nil {
		return err
	}
	defer upstream.Close()
	ip := client.RemoteAddr().(*net.TCPAddr).IP.String()
	xffNew := []byte("X-Forwarded-For: " + ip + "\r\n")
	xffAdd := []byte(", " + ip)

	req := make([]byte, 0, 8192)
	resp := make([]byte, 65536)
	var splitHead []byte

	readMore := func() error {
		if cap(req)-len(req) < 4096 {
			grown := make([]byte, len(req), 2*cap(req)+4096)
			copy(grown, req)
			req = grown
		}
		n, err := client.Read(req[len(req):cap(req)])
		req = req[:len(req)+n]
		if n == 0 && err != nil {
			return err
		}
		return nil
	}

	for {
		var h head
		for {
			var ok bool
			h, ok, err = scanHead(req)
			if err != nil {
				return err
			}
			if ok {
				break
			}
			if len(req) > 65536 {
				return errBad
			}
			if err := readMore(); err != nil {
				return err
			}
		}
		total := h.end + 4 + h.length
		for len(req) < total {
			if err := readMore(); err != nil {
				return err
			}
		}
		at, mid := h.end+2, xffNew
		if h.xffEnd >= 0 {
			at, mid = h.xffEnd, xffAdd
		}
		bufs := net.Buffers{req[:at], mid, req[at:total]}
		if _, err := bufs.WriteTo(upstream); err != nil {
			return err
		}
		// Keep whatever followed the request (pipelining).
		rest := copy(req, req[total:])
		req = req[:rest]

		left := -1
		splitHead = splitHead[:0]
		for left != 0 {
			n, err := upstream.Read(resp)
			if n == 0 {
				if err == nil {
					continue
				}
				return err
			}
			if _, err := client.Write(resp[:n]); err != nil {
				return err
			}
			if left >= 0 {
				if n > left {
					return errBad
				}
				left -= n
				continue
			}
			hb := resp[:n]
			if len(splitHead) > 0 {
				splitHead = append(splitHead, resp[:n]...)
				hb = splitHead
			}
			rh, ok, err := scanHead(hb)
			if err != nil {
				return err
			}
			if !ok {
				if len(splitHead) == 0 {
					splitHead = append(splitHead, resp[:n]...)
				}
				continue
			}
			got := len(hb) - (rh.end + 4)
			if got > rh.length {
				return errBad
			}
			left = rh.length - got
		}
	}
}

func main() {
	port, upPort := 9301, 9300
	if len(os.Args) > 1 {
		port, _ = strconv.Atoi(os.Args[1])
	}
	if len(os.Args) > 2 {
		upPort, _ = strconv.Atoi(os.Args[2])
	}
	upstreamAddr := &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: upPort}
	ln, err := net.ListenTCP("tcp", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: port})
	if err != nil {
		log.Fatal(err)
	}
	fmt.Printf("proxy listening on 127.0.0.1:%d, upstream 127.0.0.1:%d\n", port, upPort)
	for {
		c, err := ln.AcceptTCP()
		if err != nil {
			log.Fatal(err)
		}
		go serve(c, upstreamAddr)
	}
}
