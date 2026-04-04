#!/bin/bash
# Generate self-signed TLS certificates for local development.
# For production, use Let's Encrypt via certbot.

set -e

CERT_DIR="$(dirname "$0")/ssl"
mkdir -p "$CERT_DIR"

if [ -f "$CERT_DIR/fullchain.pem" ]; then
    echo "Certificates already exist in $CERT_DIR"
    exit 0
fi

echo "Generating self-signed TLS certificates..."

openssl req -x509 -nodes -days 365 -newkey rsa:2048 \
    -keyout "$CERT_DIR/privkey.pem" \
    -out "$CERT_DIR/fullchain.pem" \
    -subj "/C=US/ST=Dev/L=Local/O=IntentX/CN=localhost" \
    -addext "subjectAltName=DNS:localhost,DNS:*.localhost,IP:127.0.0.1"

echo "Certificates generated:"
echo "  $CERT_DIR/fullchain.pem"
echo "  $CERT_DIR/privkey.pem"
