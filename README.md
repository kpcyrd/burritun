# burritun

Wrap a tun interface in a tap interface. This is useful if you need to run a
tool on a tun device that depends on a layer 2 raw socket, but doesn't actually
need layer 2 access. The ethernet header is transparently stripped for egress
and added for ingress. It also emulates arp to ensure the kernel can use the
interface correctly.

![burrito logo](logo.png)

## Usage

```
# wrap tun0 into burritun0
burritun tun0 burritun0 &
# remove ip from tun device
ip a del  192.0.2.4/24 dev tun0
# add ip to tap device
ip a add 192.0.2.4/24 dev burritun0
# resume burritun
fg
```

## License

GPLv3+
