# TLS Setup Guide

## Architecture

```
Client → :443 (Nginx TLS) → :4000 (API Gateway) → :3000 (Platform)
       → :443 (Nginx TLS) → :3001 (Frontend)
       → :443 (Nginx WS)  → :3000 (WebSocket)
  :80  → 301 redirect to :443
```

All internal traffic is plain HTTP. TLS terminates at Nginx.

## Local Development (Self-Signed)

```bash
# Generate self-signed certificates
./infra/nginx/generate-dev-certs.sh

# Start everything
docker compose up -d
```

Access: `https://localhost` (browser will warn about self-signed cert)

## Production (Let's Encrypt)

### Initial Setup

```bash
# 1. Point your domain DNS to the server IP

# 2. Start nginx with HTTP only (for ACME challenge)
#    Temporarily comment out the ssl server block in nginx.conf
docker compose up -d nginx

# 3. Get initial certificate
docker compose run --rm certbot certonly \
    --webroot -w /var/www/certbot \
    -d yourdomain.com -d www.yourdomain.com \
    --email admin@yourdomain.com \
    --agree-tos --no-eff-email

# 4. Certificates are now in infra/nginx/ssl/live/yourdomain.com/
#    Update nginx.conf ssl_certificate paths:
#      ssl_certificate /etc/nginx/ssl/live/yourdomain.com/fullchain.pem;
#      ssl_certificate_key /etc/nginx/ssl/live/yourdomain.com/privkey.pem;

# 5. Restart with full TLS + auto-renewal
docker compose --profile production up -d
```

### Certificate Renewal

The certbot container auto-renews every 12 hours. After renewal, reload nginx:

```bash
docker compose exec nginx nginx -s reload
```

### Verify TLS

```bash
# Check certificate
openssl s_client -connect yourdomain.com:443 -servername yourdomain.com

# Test HTTPS
curl -v https://yourdomain.com/api/health/live

# Test HTTP redirect
curl -v http://yourdomain.com/
# Should return 301 → https://
```

## Nginx Rate Limits

Two zones are configured as an additional layer on top of application-level rate limiting:

| Zone | Rate | Burst | Applied To |
|---|---|---|---|
| `api` | 60 req/s per IP | 20 | `/api/*` |
| `auth` | 5 req/s per IP | 3 | `/api/auth/*` |

These are IP-based at the Nginx level. The API gateway has per-user Redis-backed rate limiting.

## Security Headers

Nginx adds these to all HTTPS responses:

- `Strict-Transport-Security: max-age=63072000; includeSubDomains`
- `X-Content-Type-Options: nosniff`
- `X-Frame-Options: DENY`
- `X-XSS-Protection: 1; mode=block`
- `Referrer-Policy: strict-origin-when-cross-origin`
