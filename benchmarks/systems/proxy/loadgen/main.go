// Load generator shared by every proxy run.
//
// C keep-alive connections issue N requests in total through the proxy. Every
// response is checked: status 200, the body length the upstream was started
// with, the X-Forwarded-For the upstream saw (exactly one header, carrying the
// client address appended to whatever the client sent), and the request body
// length the upstream received. Latency is measured per request.
//
// Request mix (by global request index i):
//   i%10 == 0  POST with a 100-byte body (Content-Length: 100)
//   i%7  == 0  the client sends its own X-Forwarded-For, so the proxy must append
//   otherwise  GET with no body
//
// With -pid, the proxy's resident set is sampled with `ps -o rss=` every 100 ms
// and the peak is reported.
package main

import (
	"bufio"
	"bytes"
	"flag"
	"fmt"
	"net"
	"os"
	"os/exec"
	"sort"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"
)

var (
	addr     = flag.String("addr", "127.0.0.1:9301", "proxy address")
	conns    = flag.Int("c", 256, "concurrent keep-alive connections")
	total    = flag.Int("n", 200000, "total requests")
	size     = flag.Int("size", 4096, "expected response body size")
	pid      = flag.Int("pid", 0, "proxy pid to sample RSS of (0 = none)")
	name     = flag.String("name", "proxy", "label for the result line")
	clientIP = flag.String("client-ip", "127.0.0.1", "address the proxy should see")
	verbose  = flag.Bool("v", false, "print the first errors")
)

var (
	next     atomic.Int64
	errCount atomic.Int64
	errShown atomic.Int64
)

func report(format string, a ...any) {
	errCount.Add(1)
	if *verbose && errShown.Add(1) <= 5 {
		fmt.Fprintf(os.Stderr, "error: "+format+"\n", a...)
	}
}

var postBody = bytes.Repeat([]byte("p"), 100)

func buildRequest(i int64) (req []byte, wantXFF string, wantLen int) {
	var b bytes.Buffer
	method := "GET"
	if i%10 == 0 {
		method = "POST"
	}
	fmt.Fprintf(&b, "%s /item/%d HTTP/1.1\r\nHost: upstream.local\r\nUser-Agent: proxybench\r\nAccept: */*\r\n", method, i)
	wantXFF = *clientIP
	if i%7 == 0 {
		prior := fmt.Sprintf("10.0.%d.%d", (i/256)%256, i%256)
		fmt.Fprintf(&b, "X-Forwarded-For: %s\r\n", prior)
		wantXFF = prior + ", " + *clientIP
	}
	if method == "POST" {
		fmt.Fprintf(&b, "Content-Length: %d\r\n\r\n", len(postBody))
		b.Write(postBody)
		wantLen = len(postBody)
	} else {
		b.WriteString("\r\n")
	}
	return b.Bytes(), wantXFF, wantLen
}

type conn struct {
	c  net.Conn
	br *bufio.Reader
}

func dial() (*conn, error) {
	c, err := net.Dial("tcp", *addr)
	if err != nil {
		return nil, err
	}
	return &conn{c: c, br: bufio.NewReaderSize(c, 16384)}, nil
}

// roundTrip sends one request and reads and checks one response. An error
// means the connection is no longer usable.
func (k *conn) roundTrip(i int64) error {
	req, wantXFF, wantLen := buildRequest(i)
	k.c.SetDeadline(time.Now().Add(10 * time.Second))
	if _, err := k.c.Write(req); err != nil {
		return fmt.Errorf("write: %w", err)
	}
	line, err := k.br.ReadString('\n')
	if err != nil {
		return fmt.Errorf("status line: %w", err)
	}
	if !strings.HasPrefix(line, "HTTP/1.1 200 ") {
		return fmt.Errorf("status line %q", strings.TrimSpace(line))
	}
	cl := -1
	xff, xffCount, echoLen := "", "", ""
	for {
		h, err := k.br.ReadString('\n')
		if err != nil {
			return fmt.Errorf("header: %w", err)
		}
		h = strings.TrimRight(h, "\r\n")
		if h == "" {
			break
		}
		colon := strings.IndexByte(h, ':')
		if colon < 0 {
			return fmt.Errorf("bad header %q", h)
		}
		key := strings.ToLower(h[:colon])
		val := strings.TrimSpace(h[colon+1:])
		switch key {
		case "content-length":
			cl, _ = strconv.Atoi(val)
		case "x-echo-xff":
			xff = val
		case "x-echo-xff-count":
			xffCount = val
		case "x-echo-len":
			echoLen = val
		}
	}
	if cl != *size {
		return fmt.Errorf("content-length %d, want %d", cl, *size)
	}
	if _, err := k.br.Discard(cl); err != nil {
		return fmt.Errorf("body: %w", err)
	}
	if xff != wantXFF {
		return fmt.Errorf("request %d: upstream saw X-Forwarded-For %q, want %q", i, xff, wantXFF)
	}
	if xffCount != "1" {
		return fmt.Errorf("request %d: upstream saw %s X-Forwarded-For headers", i, xffCount)
	}
	if echoLen != strconv.Itoa(wantLen) {
		return fmt.Errorf("request %d: upstream saw body of %s bytes, want %d", i, echoLen, wantLen)
	}
	return nil
}

func samplePeakRSS(stop <-chan struct{}, out chan<- int64) {
	var peak int64
	t := time.NewTicker(100 * time.Millisecond)
	defer t.Stop()
	sample := func() {
		b, err := exec.Command("ps", "-o", "rss=", "-p", strconv.Itoa(*pid)).Output()
		if err == nil {
			if v, err := strconv.ParseInt(strings.TrimSpace(string(b)), 10, 64); err == nil && v > peak {
				peak = v
			}
		}
	}
	sample()
	for {
		select {
		case <-stop:
			sample()
			out <- peak
			return
		case <-t.C:
			sample()
		}
	}
}

func main() {
	flag.Parse()
	n := int64(*total)
	lat := make([][]time.Duration, *conns)

	stop := make(chan struct{})
	peakCh := make(chan int64, 1)
	if *pid > 0 {
		go samplePeakRSS(stop, peakCh)
	}

	var wg sync.WaitGroup
	start := time.Now()
	for w := 0; w < *conns; w++ {
		wg.Add(1)
		go func(w int) {
			defer wg.Done()
			var k *conn
			mine := make([]time.Duration, 0, int(n)/(*conns)+1)
			for {
				i := next.Add(1) - 1
				if i >= n {
					break
				}
				if k == nil {
					var err error
					if k, err = dial(); err != nil {
						report("dial: %v", err)
						k = nil
						continue
					}
				}
				t0 := time.Now()
				if err := k.roundTrip(i); err != nil {
					report("%v", err)
					k.c.Close()
					k = nil
					continue
				}
				mine = append(mine, time.Since(t0))
			}
			if k != nil {
				k.c.Close()
			}
			lat[w] = mine
		}(w)
	}
	wg.Wait()
	elapsed := time.Since(start)

	var all []time.Duration
	for _, l := range lat {
		all = append(all, l...)
	}
	sort.Slice(all, func(a, b int) bool { return all[a] < all[b] })
	pct := func(p float64) float64 {
		if len(all) == 0 {
			return 0
		}
		idx := int(p * float64(len(all)-1))
		return float64(all[idx].Microseconds()) / 1000.0
	}
	peakMB := -1.0
	if *pid > 0 {
		close(stop)
		peakMB = float64(<-peakCh) / 1024.0
	}
	ok := len(all)
	rps := float64(ok) / elapsed.Seconds()
	fmt.Printf("%-8s ok=%d rps=%.0f p50=%.3fms p99=%.3fms p999=%.3fms peak_rss=%.1fMB errors=%d\n",
		*name, ok, rps, pct(0.50), pct(0.99), pct(0.999), peakMB, errCount.Load())
	if errCount.Load() != 0 || int64(ok) != n {
		os.Exit(1)
	}
}
