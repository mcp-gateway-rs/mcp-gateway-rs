# MCP Gateway


## Running

1. Generate your own public/private key for token validation
2. Run gateway
```bash 
    cargo run --bin mcp-gateway-rs -- --address 0.0.0.0:8001 --redis-port 6379 --redis-address 127.0.0.1 --token-verification-public-key assets/jwt.key.pub  --token-verification-private-key assets/jwt.key --number-of-cpus 16
```
3. Get a test JWT token
```bash
curl --request GET \
  --url http://127.0.0.1:8001/mcp-rs/admin/tokens/admin@example.com \
  --header 'accept: application/json' \  
  --header 'content-type: application/json'
```

4. Use the token to add a test user to Redis
```bash
curl --request POST \
  --url http://127.0.0.1:8001/mcp-rs/admin/userconfigs/admin@example.com \
  --header 'authorization: Bearer {{token}}' \
  --header 'content-type: application/json' \
  --data '{
  "virtualHosts": {
    "c0ffee00f001f00lf00ldeadbeefdead": {
      "backends": {
        "counter-one": {
          "url": "http://127.0.0.1:6666/mcp"
        }
      }
    }
  }
}'
```

5. Start backend service 

6. Spin up MCP Inspector to test the calls



