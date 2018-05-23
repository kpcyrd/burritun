# burritun

Wrap a tun interface in a tap interface. This is useful if you need to run a
tool on a tun device that depends on a layer 2 raw socket, but doesn't actually
need layer 2 access. The ethernet header is transparently stripped for egress
and added for ingress. It also emulates arp to ensure the kernel can use the
interface correctly.

![burrito logo](logo.png)

TODO: arp

## License

GPLv3+
