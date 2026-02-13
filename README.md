# ntfy

`ntfy` is a high-performance, lightweight, and memory-safe port of the [ntfy](https://ntfy.sh) server to Rust. It is designed to be fully compatible with the official ntfy Android and iOS apps while providing a more efficient backend for self-hosting.

## 🚀 Key Features

*   **Pub/Sub Messaging:** Simple HTTP-based publishing and subscribing.
*   **Protocol Compatibility:** Full support for SSE (Server-Sent Events), WebSockets, and JSON streams.
*   **Persistence:** SQLite-backed message caching with configurable expiry.
*   **Role-Based Access Control (RBAC):** Manage users, tokens, and glob-based access control lists (ACLs).
*   **File Attachments:** Upload and download files with integrated storage management and quotas.
*   **Mobile Push Support:** Integrated Firebase (FCM) support for Android/iOS notifications.
*   **UnifiedPush Gateway:** Built-in Matrix gateway for UnifiedPush-compatible apps.
*   **Email Integration:** Send notifications via SMTP or receive emails directly to topics.
*   **Web UI:** Includes a built-in, modern web dashboard for managing notifications.
*   **Binary-Only:** Packaged as a single optimized binary with minimal dependencies.

---

## 🛠️ Installation

### 1. Using Docker (Recommended)

The easiest way to run `ntfy` is using the official Docker image.

#### Docker Run
```bash
docker run -p 8080:80 -v ntfy-data:/var/cache/ntfy ghcr.io/eltifi/ntfy:latest
```

#### Docker Compose
Create a `docker-compose.yml` file for a fully functional public server (no authentication):
```yaml
services:
  ntfy:
    image: ghcr.io/eltifi/ntfy:latest
    container_name: ntfy
    restart: unless-stopped
    ports:
      - 8080:80
    environment:
      - NTFY_BASE_URL=http://ntfy.example.com # Replace with your URL
      - NTFY_AUTH_DEFAULT_ACCESS=read-write    # Public access (no login)
    volumes:
      - ntfy_data:/var/cache/ntfy

volumes:
  ntfy_data:
```

### 2. Building from Source

Ensure you have the latest stable [Rust toolchain](https://rustup.rs/) installed.

```bash
# Clone the repository
git clone https://github.com/eltifi/ntfy.git
cd ntfy

# Build the optimized release binary
cargo build --release

# The binary will be located at ./target/release/ntfy
./target/release/ntfy serve
```

---

## ⚙️ Configuration

`ntfy` can be configured via a YAML file or environment variables. Environment variables take precedence.

### Configuration File (`server.yml`)

Create a `server.yml` in the root directory:

```yaml
base-url: "https://ntfy.example.com"
listen-http: ":80"
cache-file: "/var/cache/ntfy/cache.db"
auth-file: "/var/lib/ntfy/auth.db"
attachment-cache-dir: "/var/cache/ntfy/attachments"

# Auth Settings
auth-default-access: "deny-all"
enable-login: true

# Firebase (for mobile push)
firebase-key-file: "/etc/ntfy/firebase-auth.json"
```

### Environment Variables

Prefix configuration keys with `NTFY_`, convert to uppercase, and replace dashes with underscores.

| Variable | Description | Default |
| :--- | :--- | :--- |
| `NTFY_BASE_URL` | Externally visible URL of your server | `http://localhost` |
| `NTFY_LISTEN_HTTP` | Interface and port to listen on | `:80` |
| `NTFY_AUTH_DEFAULT_ACCESS` | Access level for unauthenticated users | `read-write` |
| `NTFY_BEHIND_PROXY` | Set to `true` if behind Nginx/Caddy | `false` |

---

## 📖 Usage Examples

### Publishing a Notification

You can publish to any topic using a simple `POST` or `PUT` request.

```bash
curl -d "Backup completed successfully" \
     -H "Title: Server Status" \
     -H "Tags: floppy_disk,green_circle" \
     -H "Priority: high" \
     ntfy.example.com/mytopic
```

### Subscribing to a Topic

Watch for incoming notifications in real-time using SSE:

```bash
curl -s ntfy.example.com/mytopic/sse
```

### Sending Files (Attachments)

```bash
curl -T my-photo.jpg \
     -H "Filename: vacation.jpg" \
     ntfy.example.com/mytopic
```

---

## 🖥️ Command Line Interface (CLI)

The `ntfy` binary provides several built-in management tools.

### User & Permission Management

```bash
# Add an admin user
ntfy user add --role admin phil

# List all users
ntfy user list

# Grant read-only access to a specific topic for a user
# (This requires editing the auth database via CLI in the future, 
# or using the Web UI)
```

### Maintenance & Testing

```bash
# Run the built-in integration test suite
ntfy test

# Prune expired messages and attachments manually
ntfy serve --prune
```

---

## 🛡️ Production Recommendations

### Reverse Proxy (Traefik)

It is highly recommended to run `ntfy` behind a reverse proxy to handle HTTPS/TLS termination. Traefik is an excellent choice for Docker-based setups.

**Traefik Example (via Docker Labels):**

```yaml
services:
  ntfy:
    image: ghcr.io/eltifi/ntfy:latest
    # ... other configuration ...
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.ntfy.rule=Host(`ntfy.example.com`)"
      - "traefik.http.routers.ntfy.entrypoints=websecure"
      - "traefik.http.routers.ntfy.tls.certresolver=myresolver"
      - "traefik.http.services.ntfy.loadbalancer.server.port=80"
```

---

## 🤝 Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

1.  Fork the repository.
2.  Create your feature branch (`git checkout -b feature/AmazingFeature`).
3.  Commit your changes (`git commit -m 'Add some AmazingFeature'`).
4.  Push to the branch (`git push origin feature/AmazingFeature`).
5.  Open a Pull Request.

## 📄 License

Distributed under the Apache 2.0 or MIT License. See `LICENSE` for more information.