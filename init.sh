#!/bin/sh

cleaned=0
cleanup() {
  if [ $cleaned -eq 1 ]; then
    return
  fi
  cleaned=1
  echo "清理中"
  poff -a
  sleep 0.3
  echo "已斷開 PPPoE 連線"
}

trap cleanup INT
trap cleanup TERM
trap cleanup EXIT
trap cleanup STOP

cat << EOF > /etc/ppp/peers/provider
user "$PPPOE_USER"
password "$PPPOE_PASSWORD"
plugin pppoe.so eth0
noipdefault
usepeerdns
defaultroute
persist
noauth
nodetach
EOF


ip route del default
pon &
sleep 1
fail_count=0
max_retries=3
while ! ip addr show ppp0 | grep -q "inet "; do
  echo "等待 ppp0 介面取得 IP 位址..."
  fail_count=$((fail_count + 1))
    if [ $fail_count -gt $max_retries ]; then
      echo "無法取得 ppp0 介面 IP 位址，請檢查設定。"
      exit 1
    fi
  sleep 5
done
echo "ppp0 介面已取得 IP 位址：" $(ip addr show ppp0 | grep -o -P " ((\d+.){4}) ")
/usr/sbin/danted -f /etc/danted/danted1.conf -p /var/run/danted1.pid -D
wait $!