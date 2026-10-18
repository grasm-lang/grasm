package main

// The third part of the oracle: S2 (klauspost/compress), CRC-32C
// (hash/crc32) and NAT-PMP (jackpal/go-nat-pmp, against a responder on
// the loopback interface).

import (
	"bytes"
	"encoding/hex"
	"fmt"
	"hash/crc32"
	"io"
	"net"
	"strconv"
	"time"

	natpmp "github.com/jackpal/go-nat-pmp"
	"github.com/klauspost/compress/s2"
)

func answer3(r request) string {
	switch r.Op {
	case "s2.enc":
		data, _ := hex.DecodeString(r.Hex)
		var out []byte
		switch r.Args[0] {
		case "better":
			out = s2.EncodeBetter(nil, data)
		case "best":
			out = s2.EncodeBest(nil, data)
		case "snappy":
			out = s2.EncodeSnappy(nil, data)
		default:
			out = s2.Encode(nil, data)
		}
		return hex.EncodeToString(out)
	case "s2.dec":
		data, _ := hex.DecodeString(r.Hex)
		out, err := s2.Decode(nil, data)
		if err != nil {
			return "ERR"
		}
		return hex.EncodeToString(out)
	case "s2.stream.enc":
		data, _ := hex.DecodeString(r.Hex)
		size, _ := strconv.Atoi(r.Args[0])
		var buf bytes.Buffer
		opts := []s2.WriterOption{s2.WriterBlockSize(size), s2.WriterConcurrency(1)}
		if len(r.Args) > 1 && r.Args[1] == "snappy" {
			opts = append(opts, s2.WriterSnappyCompat())
		}
		if len(r.Args) > 1 && r.Args[1] == "index" {
			opts = append(opts, s2.WriterAddIndex())
		}
		w := s2.NewWriter(&buf, opts...)
		// Write in pieces so some blocks come out short.
		for len(data) > 0 {
			n := 1 + len(data)/3
			if n > len(data) {
				n = len(data)
			}
			w.Write(data[:n])
			data = data[n:]
		}
		w.Close()
		return hex.EncodeToString(buf.Bytes())
	case "s2.stream.dec":
		data, _ := hex.DecodeString(r.Hex)
		out, err := io.ReadAll(s2.NewReader(bytes.NewReader(data)))
		if err != nil {
			return "ERR"
		}
		return hex.EncodeToString(out)
	case "crc32c":
		data, _ := hex.DecodeString(r.Hex)
		return fmt.Sprint(crc32.Checksum(data, crc32.MakeTable(crc32.Castagnoli)))
	case "natpmp":
		return natpmpCase(r)
	}
	return answer4(r)
}

// natpmpCase runs one request through the client against a responder on
// 127.0.0.1:5351 that answers with the given bytes, and reports the
// request the client sent and what it made of the answer.
func natpmpCase(r request) string {
	reply, _ := hex.DecodeString(r.Hex)
	conn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 5351})
	if err != nil {
		return "ERR listen " + err.Error()
	}
	defer conn.Close()
	got := make(chan []byte, 1)
	go func() {
		buf := make([]byte, 64)
		n, from, err := conn.ReadFromUDP(buf)
		if err != nil {
			got <- nil
			return
		}
		got <- append([]byte(nil), buf[:n]...)
		conn.WriteToUDP(reply, from)
	}()
	client := natpmp.NewClientWithTimeout(net.IPv4(127, 0, 0, 1), 300*time.Millisecond)
	var result string
	if r.Args[0] == "ext" {
		res, err := client.GetExternalAddress()
		if err != nil {
			result = "ERR"
		} else {
			ip := res.ExternalIPAddress
			result = fmt.Sprintf("epoch=%d;ip=%d.%d.%d.%d", res.SecondsSinceStartOfEpoc, ip[0], ip[1], ip[2], ip[3])
		}
	} else {
		in, _ := strconv.Atoi(r.Args[2])
		ext, _ := strconv.Atoi(r.Args[3])
		life, _ := strconv.Atoi(r.Args[4])
		res, err := client.AddPortMapping(r.Args[1], in, ext, life)
		if err != nil {
			result = "ERR"
		} else {
			result = fmt.Sprintf("epoch=%d;in=%d;out=%d;life=%d", res.SecondsSinceStartOfEpoc, res.InternalPort,
				res.MappedExternalPort, res.PortMappingLifetimeInSeconds)
		}
	}
	req := <-got
	return hex.EncodeToString(req) + "|" + result
}
