from alpine:latest

run apk add --no-cache iproute2
run apk add --no-cache iptables

workdir /bin
copy gost .

env TZ=Asia/Taipei

cmd ["./ppproxy"]