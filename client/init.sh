#! /bin/sh

if [ -z "$SERVER_IP" ]; then
  echo "SERVER_IP is not set"
  exit 1
fi
if [ -z "$RELAY_PORT" ]; then
  echo "RELAY_PORT is not set"
  exit 1
fi
if [ -z "$SERVICE_ID" ]; then
  echo "SERVICE_ID is not set"
  exit 1
fi

rand

LOCAL_IP="192.168.10$SERVICE_ID.$(shuf -i 1-254 -n 1)"
TARGET_PORT="888$SERVICE_ID"


GATEWAY=$(ip route show 0.0.0.0/0 | cut -d\  -f3 | head -n 1)

echo "========== Client Config =========="
echo "Server IP      : $SERVER_IP"
echo "Relay port     : $RELAY_PORT"
echo "Service ID     : $SERVICE_ID"
echo "Target port    : $TARGET_PORT"
echo "Default gateway: $GATEWAY"
echo "Local IP       : $LOCAL_IP"
echo "==================================="


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

gost