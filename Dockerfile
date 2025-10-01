from alpine:latest

run apk add --no-cache iproute2 ppp-pppoe

cmd ["pppoe-discovery", "-I", "eth0"]