from alpine:latest

run apk add --no-cache iproute2
run apk add --no-cache iptables

workdir /bin
copy gost .
workdir /app
copy client/init.sh .

env TZ=Asia/Taipei

healthcheck \
  --interval=30s \
  --timeout=10s \
  --retries=3 \
  --start-period=2s \
  CMD sh -c "ip link show | grep -q tun || exit 1"

cmd ["sh", "init.sh"]