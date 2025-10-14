FROM alpine:latest

RUN apk add --no-cache iproute2
RUN apk add --no-cache iptables
RUN apk add --no-cache tzdata
RUN apk add --no-cache curl

WORKDIR /bin
COPY gost .
WORKDIR /app
COPY client/init.sh .

ENV TZ=Asia/Taipei

HEALTHCHECK \
  --interval=30s \
  --timeout=10s \
  --retries=3 \
  --start-period=2s \
  CMD sh -c "curl -m 1 -I 1.1.1.1 || exit 1"

CMD ["sh", "init.sh"]