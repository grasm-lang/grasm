// Load generator for the chat broadcast benchmark. Written once, used for all
// three servers.
//
// It opens -conns connections, joins client k to channel k % C (C = conns /
// chsize), waits for every join to be acknowledged, then has the first
// -senders clients send -msgs messages each at -rate messages/second. Every
// message carries the sender id, a sequence number and a monotonic send time
// in nanoseconds; every receiver checks the message belongs to its channel and
// was not its own, and records the latency.
//
// Protocol: "J <channel>\n" -> "OK\n"; then "M <sender> <seq> <nanos>\n" lines,
// broadcast by the server to the channel's other members.
package main

import (
	"bufio"
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

var base = time.Now()

func nowNanos() int64 { return int64(time.Since(base)) }

type client struct {
	id   int
	ch   int
	conn net.Conn
	rd   *bufio.Reader
	lat  []int32 // microseconds
}

func main() {
	addrs := flag.String("addrs", "127.0.0.1:9000", "server address(es), comma separated; connection i dials addrs[i % len]")
	conns := flag.Int("conns", 10000, "client connections")
	chsize := flag.Int("chsize", 1000, "clients per channel (channels = conns/chsize)")
	sendersF := flag.Int("senders", -1, "clients that send (default conns/10)")
	msgs := flag.Int("msgs", 10, "messages per sender")
	rate := flag.Float64("rate", 1.0, "messages per second per sender")
	pid := flag.Int("pid", 0, "server pid whose RSS is sampled every 100 ms")
	name := flag.String("name", "server", "label for the result line")
	drain := flag.Duration("drain", 60*time.Second, "give up this long after the last delivery stops arriving")
	dialers := flag.Int("dialers", 64, "connections dialled concurrently")
	laddrs := flag.String("laddrs", "", "local source IPs, comma separated; connection i binds laddrs[i % len] (each gives ~16k more ephemeral ports; on macOS add them first with `sudo ifconfig lo0 alias 127.0.0.2`)")
	flag.Parse()

	N := *conns
	C := N / *chsize
	if C < 1 {
		C = 1
	}
	M := *sendersF
	if M < 0 {
		M = N / 10
	}
	if M > N {
		M = N
	}
	if M < 1 {
		M = 1
	}
	targets := strings.Split(*addrs, ",")
	var sources []string
	if *laddrs != "" {
		sources = strings.Split(*laddrs, ",")
	}

	// Peak RSS of the server.
	var peakKB atomic.Int64
	stopRSS := make(chan struct{})
	rssDone := make(chan struct{})
	go func() {
		defer close(rssDone)
		if *pid == 0 {
			return
		}
		sample := func() {
			out, err := exec.Command("ps", "-o", "rss=", "-p", strconv.Itoa(*pid)).Output()
			if err == nil {
				if kb, err := strconv.ParseInt(strings.TrimSpace(string(out)), 10, 64); err == nil && kb > peakKB.Load() {
					peakKB.Store(kb)
				}
			}
		}
		t := time.NewTicker(100 * time.Millisecond)
		defer t.Stop()
		for {
			sample()
			select {
			case <-stopRSS:
				sample()
				return
			case <-t.C:
			}
		}
	}()

	// Connect and join.
	clients := make([]*client, N)
	{
		t0 := time.Now()
		var wg sync.WaitGroup
		next := atomic.Int64{}
		var failed atomic.Int64
		var firstErr atomic.Value
		for w := 0; w < *dialers; w++ {
			wg.Add(1)
			go func() {
				defer wg.Done()
				for {
					i := int(next.Add(1) - 1)
					if i >= N {
						return
					}
					src := ""
					if len(sources) > 0 {
						src = sources[i%len(sources)]
					}
					c, err := join(targets[i%len(targets)], src, i, i%C)
					if err != nil {
						failed.Add(1)
						firstErr.CompareAndSwap(nil, err.Error())
						continue
					}
					clients[i] = c
				}
			}()
		}
		wg.Wait()
		if failed.Load() > 0 {
			fmt.Fprintf(os.Stderr, "loadgen: %d of %d connections failed to join (first: %v)\n", failed.Load(), N, firstErr.Load())
			os.Exit(2)
		}
		fmt.Fprintf(os.Stderr, "loadgen: %d connections joined %d channels in %.2fs\n", N, C, time.Since(t0).Seconds())
	}

	// Expected deliveries: each sender's message reaches every other member of its channel.
	size := func(c int) int64 {
		s := int64(N / C)
		if c < N%C {
			s++
		}
		return s
	}
	var expected int64
	for s := 0; s < M; s++ {
		expected += int64(*msgs) * (size(s%C) - 1)
	}

	var delivered, bad atomic.Int64
	var lastDelivery atomic.Int64
	var rwg sync.WaitGroup
	for _, c := range clients {
		rwg.Add(1)
		go func(c *client) {
			defer rwg.Done()
			for {
				line, err := c.rd.ReadSlice('\n')
				if err != nil {
					return
				}
				now := nowNanos()
				f := strings.Fields(string(line))
				if len(f) != 4 || f[0] != "M" {
					bad.Add(1)
					continue
				}
				sender, _ := strconv.Atoi(f[1])
				ts, _ := strconv.ParseInt(f[3], 10, 64)
				if sender == c.id || sender%C != c.ch {
					bad.Add(1)
					continue
				}
				c.lat = append(c.lat, int32((now-ts)/1000))
				delivered.Add(1)
				lastDelivery.Store(now)
			}
		}(c)
	}

	// Send.
	interval := time.Duration(float64(time.Second) / *rate)
	start := nowNanos()
	var swg sync.WaitGroup
	for s := 0; s < M; s++ {
		swg.Add(1)
		go func(c *client, s int) {
			defer swg.Done()
			// Spread the senders evenly across one interval.
			offset := time.Duration(int64(interval) * int64(s) / int64(M))
			buf := make([]byte, 0, 64)
			for j := 0; j < *msgs; j++ {
				due := start + int64(offset) + int64(j)*int64(interval)
				if d := due - nowNanos(); d > 0 {
					time.Sleep(time.Duration(d))
				}
				buf = buf[:0]
				buf = append(buf, "M "...)
				buf = strconv.AppendInt(buf, int64(c.id), 10)
				buf = append(buf, ' ')
				buf = strconv.AppendInt(buf, int64(j), 10)
				buf = append(buf, ' ')
				buf = strconv.AppendInt(buf, nowNanos(), 10)
				buf = append(buf, '\n')
				if _, err := c.conn.Write(buf); err != nil {
					fmt.Fprintf(os.Stderr, "loadgen: send from %d failed: %v\n", c.id, err)
					return
				}
			}
		}(clients[s], s)
	}
	swg.Wait()
	sendEnd := nowNanos()

	// Wait for every delivery, or for deliveries to stop arriving.
	for delivered.Load() < expected {
		last := lastDelivery.Load()
		if last < sendEnd {
			last = sendEnd
		}
		if time.Duration(nowNanos()-last) > *drain {
			break
		}
		time.Sleep(10 * time.Millisecond)
	}
	// Anything extra (duplicates) would show up now.
	time.Sleep(200 * time.Millisecond)
	end := lastDelivery.Load()
	close(stopRSS)
	<-rssDone

	// Reset rather than close: 10,000 sockets in TIME_WAIT would hold most
	// of the ephemeral port range for the next server's run.
	for _, c := range clients {
		if tc, ok := c.conn.(*net.TCPConn); ok {
			tc.SetLinger(0)
		}
		c.conn.Close()
	}
	rwg.Wait()

	var all []int32
	for _, c := range clients {
		all = append(all, c.lat...)
	}
	sort.Slice(all, func(i, j int) bool { return all[i] < all[j] })
	pct := func(p float64) float64 {
		if len(all) == 0 {
			return 0
		}
		i := int(p * float64(len(all)-1))
		return float64(all[i]) / 1000.0
	}
	got := delivered.Load()
	ok := got == expected && bad.Load() == 0
	secs := float64(end-start) / 1e9
	thr := 0.0
	if secs > 0 {
		thr = float64(got) / secs
	}
	fmt.Printf("RESULT name=%s conns=%d channels=%d senders=%d msgs=%d rate=%g deliveries=%d expected=%d bad=%d exact=%v seconds=%.3f throughput=%.0f p50_ms=%.3f p99_ms=%.3f p999_ms=%.3f peak_rss_mb=%.1f\n",
		*name, N, C, M, *msgs, *rate, got, expected, bad.Load(), ok, secs, thr, pct(0.50), pct(0.99), pct(0.999), float64(peakKB.Load())/1024.0)
	if !ok {
		os.Exit(1)
	}
}

func join(addr, src string, id, ch int) (*client, error) {
	var conn net.Conn
	var err error
	d := net.Dialer{Timeout: 5 * time.Second}
	if src != "" {
		d.LocalAddr = &net.TCPAddr{IP: net.ParseIP(src)}
	}
	for attempt := 0; attempt < 20; attempt++ {
		conn, err = d.Dial("tcp", addr)
		if err == nil {
			break
		}
		time.Sleep(time.Duration(attempt+1) * 20 * time.Millisecond)
	}
	if err != nil {
		return nil, err
	}
	if _, err := fmt.Fprintf(conn, "J %d\n", ch); err != nil {
		conn.Close()
		return nil, err
	}
	rd := bufio.NewReaderSize(conn, 64*1024)
	conn.SetReadDeadline(time.Now().Add(30 * time.Second))
	line, err := rd.ReadString('\n')
	if err != nil || line != "OK\n" {
		conn.Close()
		return nil, fmt.Errorf("join %d: %q %v", id, line, err)
	}
	conn.SetReadDeadline(time.Time{})
	return &client{id: id, ch: ch, conn: conn, rd: rd}, nil
}
