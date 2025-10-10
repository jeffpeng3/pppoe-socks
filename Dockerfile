from rust as builder

workdir /app
run rustup target add x86_64-unknown-linux-musl

run cargo init ppproxy
workdir /app/ppproxy
copy Cargo.toml Cargo.lock ./
run cargo build --release --target x86_64-unknown-linux-musl

copy src ./src
run touch -a -m ./src/main.rs
run cargo build --release --target x86_64-unknown-linux-musl


from alpine:latest

run apk add --no-cache rp-pppoe
run apk add --no-cache ppp-pppoe
run apk add --no-cache nftables
run apk add --no-cache tzdata
run apk add --no-cache iproute2
# run apk add --no-cache curl
# run apk add curl iputils-ping

workdir /app
add rt_tables /etc/iproute2/rt_tables
add nftables.conf /etc/nftables.conf
copy gost ./gost
copy --from=builder /app/ppproxy/target/x86_64-unknown-linux-musl/release/ppproxy .

env TZ=Asia/Taipei
env docker=true

cmd ["./ppproxy"]