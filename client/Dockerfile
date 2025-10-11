from alpine:latest

run apk add --no-cache iproute2
run apk add --no-cache iptables

workdir /bin
copy gost .
workdir /app
copy client/init.sh .

env TZ=Asia/Taipei

cmd ["sh", "init.sh"]