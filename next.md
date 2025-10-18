# Next: Fix Real Bugs Hidden by Test Timeouts

## Problem

Tests need 5-10 second timeouts on a fast computer. This is hiding real bugs. Logs show:

```
[2025-10-18T13:45:33Z ERROR] Connection error: Failed to send heartbeat:
    Failed to send data: IO(Custom { kind: BrokenPipe, error: "channel closed" })
[2025-10-18T13:45:34Z INFO] Reconnecting in 5 seconds...
```

## Root Causes

1. **Client Disconnection**: Client daemon connects successfully, but then:
   - Test sends LocalCommand (ListClients/WatchDirectory)
   - This creates a NEW SSH connection to the server
   - Something about this new connection breaks the client daemon's persistent connection
   - Client daemon has to reconnect

2. **Slow Reconnect**: Client reconnect delay is 5 seconds minimum
   - This is the actual bottleneck, not the sync itself
   - Tests wait 5+ seconds for client to reconnect after being broken

3. **Hidden Failures**: Long timeouts (5-10 seconds) hide the fact that client is disconnecting

## Investigation Needed

1. **Why does LocalCommand connection break client daemon connection?**
   - Both use same SSH port
   - Are they interfering with each other?
   - Does server drop old connection when new one arrives?
   - Check server-side connection handling

2. **Check authorized_keys or SSH agent issues**
   - Multiple connections with same key?
   - Server dropping duplicate connections?

3. **Review ClientRegistry in ssh_server.rs**
   - Does it properly handle multiple connections from same user?
   - Is it unregistering client when control command connects?

## Solution Options

### Option A: Fix Connection Interference (PREFERRED)
- Investigate why control commands break client daemon connections
- Ensure server can handle multiple SSH connections from same user/key
- May need to distinguish connection types at SSH level

### Option B: Reduce Reconnect Delay for Tests
- Add environment variable or test-only flag to reduce reconnect delay
- Client daemon could use 100ms reconnect in tests vs 5s in production
- DOWNSIDE: Still hiding the real bug

### Option C: Reuse Client Connection for Control Commands
- Tests could send control commands through the client daemon's connection
- Would require protocol changes
- More complex, but would eliminate the interference

## Immediate Action

1. Reduce all test timeouts to 2 seconds to expose bugs faster
2. Add debug logging when client disconnects
3. Add debug logging when new SSH connections are accepted
4. Run test and analyze exactly when/why client disconnects

## Expected Outcome

Tests should work with ~100ms timeouts if:
- Client stays connected throughout test
- Sync happens immediately when file changes
- No reconnection delays
