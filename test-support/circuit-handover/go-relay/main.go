// Loopback-only transport fixture. This intentionally mocks relay admission;
// it is NOT a Hagall authentication/provider implementation or deployable server.
package main

import (
	"bufio"
	"encoding/binary"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"os"
	"sync/atomic"
	"time"

	libp2p "github.com/libp2p/go-libp2p"
	"github.com/libp2p/go-libp2p/core/network"
	"github.com/libp2p/go-libp2p/core/peer"
	pb "github.com/libp2p/go-libp2p/p2p/protocol/circuitv2/pb"
	"github.com/libp2p/go-libp2p/p2p/protocol/circuitv2/relay"
	ma "github.com/multiformats/go-multiaddr"
)

type fixture struct {
	opened, closed, active, peak, reservations, admissions, rejected atomic.Int64
	deny                                                             atomic.Bool
	delay                                                            atomic.Int64
}

func (f *fixture) RelayStatus(bool) {}
func (f *fixture) ConnectionOpened() {
	f.opened.Add(1)
	n := f.active.Add(1)
	for p := f.peak.Load(); n > p; p = f.peak.Load() {
		if f.peak.CompareAndSwap(p, n) {
			break
		}
	}
}
func (f *fixture) ConnectionClosed(time.Duration) { f.closed.Add(1); f.active.Add(-1) }
func (f *fixture) ConnectionRequestHandled(s pb.Status) {
	if s != pb.Status_OK {
		f.rejected.Add(1)
	}
}
func (f *fixture) ReservationAllowed(renewal bool) {
	if !renewal {
		f.reservations.Add(1)
	}
}
func (f *fixture) ReservationClosed(int)                   {}
func (f *fixture) ReservationRequestHandled(pb.Status)     {}
func (f *fixture) BytesTransferred(int)                    {}
func (f *fixture) AllowReserve(peer.ID, ma.Multiaddr) bool { return true }
func (f *fixture) AllowConnect(peer.ID, ma.Multiaddr, peer.ID) bool {
	if d := f.delay.Load(); d > 0 {
		time.Sleep(time.Duration(d) * time.Millisecond)
	}
	return !f.deny.Load()
}
func (f *fixture) stats() map[string]int64 {
	return map[string]int64{"opened": f.opened.Load(), "closed": f.closed.Load(), "active": f.active.Load(), "peak": f.peak.Load(), "reservations": f.reservations.Load(), "admissions": f.admissions.Load(), "rejected": f.rejected.Load()}
}
func main() {
	duration := flag.Duration("duration", 6*time.Second, "test-only circuit lifetime")
	flag.Parse()
	if *duration <= 0 || *duration > time.Minute {
		panic("fixture duration out of bounds")
	}
	h, err := libp2p.New(libp2p.ListenAddrStrings("/ip4/127.0.0.1/tcp/0"), libp2p.DisableRelay(),
		libp2p.AddrsFactory(func(addrs []ma.Multiaddr) []ma.Multiaddr {
			// Go omits loopback addresses in reservation responses. The Rust
			// test resolver maps this advertised test name back to loopback.
			for _, addr := range addrs {
				port, err := addr.ValueForProtocol(ma.P_TCP)
				if err == nil {
					return []ma.Multiaddr{ma.StringCast("/dns4/handover.relay.auki-p2p.dev/tcp/" + port)}
				}
			}
			return nil
		}))
	if err != nil {
		panic(err)
	}
	defer h.Close()
	f := &fixture{}
	resources := relay.DefaultResources()
	resources.Limit = &relay.RelayLimit{Duration: *duration, Data: 64 * 1024 * 1024}
	resources.MaxCircuits = 32
	r, err := relay.New(h, relay.WithResources(resources), relay.WithACL(f), relay.WithMetricsTracer(f))
	if err != nil {
		panic(err)
	}
	defer r.Close()
	h.SetStreamHandler("/auki-p2p/relay-auth/1", func(s network.Stream) {
		defer s.Close()
		_ = s.SetDeadline(time.Now().Add(3 * time.Second))
		var n uint32
		if binary.Read(s, binary.BigEndian, &n) != nil || n == 0 || n > 65536 {
			_ = s.Reset()
			return
		}
		// Consume but never print or retain the fixture credential.
		if _, err := io.CopyN(io.Discard, s, int64(n)); err != nil {
			_ = s.Reset()
			return
		}
		f.admissions.Add(1)
		response, _ := json.Marshal(map[string]any{"accepted": true, "accepted_until": time.Now().Add(20 * time.Second).UTC().Format(time.RFC3339)})
		if binary.Write(s, binary.BigEndian, uint32(len(response))) != nil {
			return
		}
		_, _ = s.Write(response)
	})
	port, err := h.Addrs()[0].ValueForProtocol(ma.P_TCP)
	if err != nil {
		panic(err)
	}
	enc := json.NewEncoder(os.Stdout)
	_ = enc.Encode(map[string]any{"peer_id": h.ID().String(), "port": port, "duration_secs": int64(duration.Seconds())})
	scanner := bufio.NewScanner(os.Stdin)
	for scanner.Scan() {
		switch scanner.Text() {
		case "stats":
		case "deny":
			f.deny.Store(true)
		case "allow":
			f.deny.Store(false)
			f.delay.Store(0)
		case "delay":
			f.delay.Store(400)
		default:
			fmt.Fprintln(os.Stderr, "unknown fixture command")
			return
		}
		_ = enc.Encode(f.stats())
	}
}
