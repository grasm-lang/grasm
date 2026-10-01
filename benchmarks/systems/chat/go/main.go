// Chat broadcast server, Go: a goroutine per connection.
//
// Protocol (identical in all three servers): "J <channel>\n" first, answered
// with "OK\n"; every later line is broadcast verbatim to the channel's other
// members. Per read, everything up to the last complete newline goes out as
// one write per peer; a partial line waits for the rest.
//
// Each client has a reader goroutine and a writer goroutine fed by an
// unbounded queue, so a slow receiver delays only itself and nothing is
// dropped (the benchmark checks every delivery).
//
// Usage: server [port]     (default 9000; prints "listening <port>")
package main

import (
	"bufio"
	"bytes"
	"fmt"
	"net"
	"os"
	"strconv"
	"strings"
	"sync"
)

type client struct {
	conn net.Conn
	mu   sync.Mutex
	cond *sync.Cond
	q    [][]byte
	done bool
}

func (c *client) enqueue(b []byte) {
	c.mu.Lock()
	c.q = append(c.q, b)
	c.mu.Unlock()
	c.cond.Signal()
}

func (c *client) writer() {
	var batch [][]byte
	for {
		c.mu.Lock()
		for len(c.q) == 0 && !c.done {
			c.cond.Wait()
		}
		if c.done {
			c.mu.Unlock()
			return
		}
		batch, c.q = c.q, batch[:0]
		c.mu.Unlock()
		bufs := net.Buffers(batch)
		if _, err := bufs.WriteTo(c.conn); err != nil {
			return
		}
		for i := range batch {
			batch[i] = nil
		}
	}
}

type room struct {
	mu      sync.RWMutex
	members map[*client]struct{}
}

type server struct {
	mu    sync.Mutex
	rooms map[int]*room
}

func (s *server) room(id int) *room {
	s.mu.Lock()
	defer s.mu.Unlock()
	r := s.rooms[id]
	if r == nil {
		r = &room{members: map[*client]struct{}{}}
		s.rooms[id] = r
	}
	return r
}

func (s *server) serve(conn net.Conn) {
	defer conn.Close()
	if tc, ok := conn.(*net.TCPConn); ok {
		tc.SetNoDelay(true)
	}
	rd := bufio.NewReaderSize(conn, 4096)
	line, err := rd.ReadString('\n')
	if err != nil {
		return
	}
	f := strings.Fields(line)
	ch := 0
	if len(f) >= 2 {
		ch, _ = strconv.Atoi(f[1])
	}
	c := &client{conn: conn}
	c.cond = sync.NewCond(&c.mu)
	r := s.room(ch)
	r.mu.Lock()
	r.members[c] = struct{}{}
	r.mu.Unlock()
	go c.writer()
	defer func() {
		r.mu.Lock()
		delete(r.members, c)
		r.mu.Unlock()
		c.mu.Lock()
		c.done = true
		c.mu.Unlock()
		c.cond.Signal()
	}()
	if _, err := conn.Write([]byte("OK\n")); err != nil {
		return
	}

	// Whatever followed the join in the same read is already in rd's buffer.
	var pending []byte
	if n := rd.Buffered(); n > 0 {
		b, _ := rd.Peek(n)
		pending = append(pending, b...)
	}
	buf := make([]byte, 64*1024)
	for {
		if last := bytes.LastIndexByte(pending, '\n'); last >= 0 {
			out := make([]byte, last+1)
			copy(out, pending[:last+1])
			pending = append(pending[:0], pending[last+1:]...)
			r.mu.RLock()
			for m := range r.members {
				if m != c {
					m.enqueue(out)
				}
			}
			r.mu.RUnlock()
		}
		n, err := conn.Read(buf)
		if n > 0 {
			pending = append(pending, buf[:n]...)
		}
		if err != nil {
			return
		}
	}
}

func main() {
	port := "9000"
	if len(os.Args) > 1 {
		port = os.Args[1]
	}
	ln, err := net.Listen("tcp", "127.0.0.1:"+port)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	fmt.Printf("listening %d\n", ln.Addr().(*net.TCPAddr).Port)
	s := &server{rooms: map[int]*room{}}
	for {
		conn, err := ln.Accept()
		if err != nil {
			continue
		}
		go s.serve(conn)
	}
}
