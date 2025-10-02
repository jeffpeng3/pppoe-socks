from ubuntu

run apt-get update
run apt-get install -y pppoe pppoeconf 
run apt-get install -y nano
run apt-get install -y curl  iputils-ping
run apt-get install -y dante-server

add init.sh /init.sh
run chmod +x /init.sh

add danted /etc/danted/

cmd ["/init.sh"]
# cmd ["tail", "-f", "/dev/null"]