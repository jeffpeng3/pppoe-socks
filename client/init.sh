#! /bin/sh

if [ -z "$SERVER_IP" ]; then
  echo "SERVER_IP is not set"
  exit 1
fi
if [ -z "$RELAY_PORT" ]; then
  echo "RELAY_PORT is not set"
  exit 1
fi
if [ -z "$TARGET_PORT" ]; then
  echo "TARGET_PORT is not set"
  exit 1
fi
if [ -z "$LOCAL_IP" ]; then
  echo "LOCAL_IP is not set"
  exit 1
fi

GATEWAY=$(ip route show 0.0.0.0/0 | cut -d\  -f3 | head -n 1)
echo "Default gateway: $GATEWAY"

ip r $SERVER_IP/32 via $GATEWAY dev eth0
ip r d default

cat >> gost.yml << EOF
services:
  - name: tun-service
    addr: :0
    handler:
      type: tun
      chain: chain-0
      metadata:
        bufferSize: "65535"
        keepAlive: "true"
    listener:
      type: tun
      metadata:
        net: $LOCAL_IP/24
        route: 0.0.0.0/0
    forwarder:
      nodes:
        - name: target-0
          addr: :$TARGET_PORT
chains:
  - name: chain-0
    hops:
      - name: hop-0
        nodes:
          - name: node-0
            addr: $SERVER_IP:$RELAY_PORT
            connector:
              type: relay
            dialer:
              type: tcp
              tls:
                serverName: $SERVER_IP
log:
  output: stdout
  level: info
  format: text

EOF

/bin/gost