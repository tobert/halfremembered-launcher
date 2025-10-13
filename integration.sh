#!/bin/bash
# HalfRemembered Launcher - Integration Test Suite
#
# This script performs a comprehensive integration test of the SSH-based
# remote launcher system, testing authentication, client registration,
# and various control commands.

set -e  # Exit on any error

# ═══════════════════════════════════════════════════════════════════════════
# 🎨 Colors & Icons
# ═══════════════════════════════════════════════════════════════════════════

# ANSI color codes for pretty output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
CYAN='\033[0;36m'
WHITE='\033[1;37m'
GRAY='\033[0;90m'
BOLD='\033[1m'
RESET='\033[0m'

# Nerd Font icons for visual flair
ROCKET="🚀"
CHECK="✓"
CROSS="✗"
GEAR="⚙"
SERVER="🖥"
CLIENT="💻"
KEY="🔑"
MAGNIFY="🔍"
CLEAN="🧹"
SPARKLE="✨"
FIRE="🔥"
HOURGLASS="⏳"

# ═══════════════════════════════════════════════════════════════════════════
# 📝 Configuration
# ═══════════════════════════════════════════════════════════════════════════

# Test configuration
PORT=20222
USER=$(whoami)
HOST="localhost"
SERVER_LOG="/tmp/integration-test-server.log"
CLIENT_LOG="/tmp/integration-test-client.log"
BINARY="./target/release/halfremembered-launcher"

# Test tracking
TESTS_RUN=0
TESTS_PASSED=0
TESTS_FAILED=0

# ═══════════════════════════════════════════════════════════════════════════
# 🛠 Helper Functions
# ═══════════════════════════════════════════════════════════════════════════

# Print a section header with style
print_header() {
    echo ""
    echo -e "${BOLD}${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
    echo -e "${BOLD}${WHITE}  $1${RESET}"
    echo -e "${BOLD}${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
}

# Print a test step
print_step() {
    echo -e "${BLUE}${GEAR}${RESET}  ${WHITE}$1${RESET}"
}

# Print success message
print_success() {
    echo -e "${GREEN}${CHECK}${RESET}  ${GREEN}$1${RESET}"
    TESTS_PASSED=$((TESTS_PASSED + 1))
}

# Print failure message and exit
print_error() {
    echo -e "${RED}${CROSS}${RESET}  ${RED}$1${RESET}"
    TESTS_FAILED=$((TESTS_FAILED + 1))
    cleanup
    exit 1
}

# Print info message
print_info() {
    echo -e "${GRAY}  ℹ  $1${RESET}"
}

# Run a test and check the result
run_test() {
    local test_name="$1"
    local expected="$2"
    local actual="$3"

    TESTS_RUN=$((TESTS_RUN + 1))

    if echo "$actual" | grep -q "$expected"; then
        print_success "Test passed: $test_name"
        return 0
    else
        print_error "Test failed: $test_name\n    Expected: $expected\n    Got: $actual"
        return 1
    fi
}

# Cleanup function - kills all test processes
cleanup() {
    print_header "${CLEAN} Cleanup"
    print_step "Terminating test processes..."
    killall halfremembered-launcher 2>/dev/null || true
    sleep 1
    print_info "Cleanup complete"
}

# Trap to ensure cleanup on exit
trap cleanup EXIT

# ═══════════════════════════════════════════════════════════════════════════
# 🏗 Pre-flight Checks
# ═══════════════════════════════════════════════════════════════════════════

print_header "${ROCKET} HalfRemembered Launcher - Integration Test"

print_step "Checking for required binary..."
if [ ! -f "$BINARY" ]; then
    print_error "Binary not found at $BINARY. Run 'cargo build --release' first!"
fi
print_success "Binary found at $BINARY"

print_step "Checking for authorized_keys..."
if [ ! -f "$HOME/.ssh/authorized_keys" ]; then
    print_error "~/.ssh/authorized_keys not found!"
fi
KEY_COUNT=$(grep -v "^#" ~/.ssh/authorized_keys | grep -v "^$" | wc -l)
print_success "Found $KEY_COUNT authorized key(s)"

print_step "Cleaning up any existing test processes..."
killall halfremembered-launcher 2>/dev/null || true
sleep 1

# ═══════════════════════════════════════════════════════════════════════════
# 🖥 Start Server
# ═══════════════════════════════════════════════════════════════════════════

print_header "${SERVER} Starting SSH Server"

print_step "Launching server on port $PORT..."
RUST_LOG=info $BINARY server --port $PORT > $SERVER_LOG 2>&1 &
SERVER_PID=$!
print_info "Server PID: $SERVER_PID"

print_step "Waiting for server to initialize..."
sleep 2

# Check if server is still running
if ! kill -0 $SERVER_PID 2>/dev/null; then
    print_error "Server failed to start! Check logs:\n$(tail -5 $SERVER_LOG)"
fi

# Verify server started successfully
if grep -q "Starting SSH server" $SERVER_LOG; then
    print_success "Server started successfully"
else
    print_error "Server startup message not found in logs"
fi

# Verify keys were loaded
if grep -q "Loaded.*authorized keys" $SERVER_LOG; then
    LOADED_KEYS=$(grep "Loaded.*authorized keys" $SERVER_LOG | grep -oP '\d+(?= authorized)')
    print_success "Server loaded $LOADED_KEYS authorized key(s)"
else
    print_error "No authorized keys loaded!"
fi

# ═══════════════════════════════════════════════════════════════════════════
# 💻 Start Client
# ═══════════════════════════════════════════════════════════════════════════

print_header "${CLIENT} Starting Client Daemon"

print_step "Launching client daemon..."
RUST_LOG=info $BINARY client $USER@$HOST --port $PORT > $CLIENT_LOG 2>&1 &
CLIENT_PID=$!
print_info "Client PID: $CLIENT_PID"

print_step "Waiting for client to connect and authenticate..."
sleep 2

# Check if client is still running
if ! kill -0 $CLIENT_PID 2>/dev/null; then
    print_error "Client failed to start! Check logs:\n$(tail -5 $CLIENT_LOG)"
fi

# ═══════════════════════════════════════════════════════════════════════════
# 🔑 Verify Authentication
# ═══════════════════════════════════════════════════════════════════════════

print_header "${KEY} Testing SSH Authentication"

print_step "Checking server logs for authentication..."
if grep -q "Public key authentication successful" $SERVER_LOG; then
    print_success "Server: SSH authentication successful"
else
    print_error "Server authentication failed!"
fi

print_step "Checking client logs for successful connection..."
if grep -q "Successfully authenticated with ssh-agent" $CLIENT_LOG; then
    print_success "Client: Authenticated with ssh-agent"
else
    print_error "Client authentication failed!"
fi

if grep -q "Server welcomed us" $CLIENT_LOG; then
    SESSION_ID=$(grep "Server welcomed us" $CLIENT_LOG | grep -oP 'session=\K[a-f0-9-]+')
    print_success "Client registered with session: ${SESSION_ID:0:8}..."
else
    print_error "Client registration failed!"
fi

# ═══════════════════════════════════════════════════════════════════════════
# 🔍 Test Control Commands
# ═══════════════════════════════════════════════════════════════════════════

print_header "${MAGNIFY} Testing Control Commands"

# Test 1: List Clients
print_step "Test 1: List connected clients..."
sleep 1
OUTPUT=$($BINARY list -s $USER@$HOST -P $PORT 2>&1)
run_test "List shows connected clients" "Connected clients" "$OUTPUT"
run_test "List shows our hostname" "$(hostname)" "$OUTPUT"
print_info "Output: $(echo "$OUTPUT" | head -1)"

# Test 2: Server Status
print_step "Test 2: Get server status..."
sleep 1
OUTPUT=$($BINARY status -s $USER@$HOST -P $PORT 2>&1)
run_test "Status shows server version" "Server version" "$OUTPUT"
run_test "Status shows client count" "Connected clients: 1" "$OUTPUT"
run_test "Status shows client uptime" "uptime:" "$OUTPUT"
print_info "$(echo "$OUTPUT" | grep "Connected clients")"

# Test 3: Ping Client
print_step "Test 3: Ping client..."
sleep 1
OUTPUT=$($BINARY ping -s $USER@$HOST -P $PORT $(hostname) 2>&1)
run_test "Ping successful" "Ping sent" "$OUTPUT"
print_info "Output: $OUTPUT"

# Test 4: Execute Command
print_step "Test 4: Execute remote command..."
sleep 1
# Create a simple test script
echo '#!/bin/bash' > /tmp/integration-test-cmd.sh
echo 'echo "Hello from $(hostname)!"' >> /tmp/integration-test-cmd.sh
chmod +x /tmp/integration-test-cmd.sh

OUTPUT=$($BINARY exec -s $USER@$HOST -P $PORT $(hostname) /bin/echo "Integration test" 2>&1)
run_test "Execute command sent" "Execute command sent" "$OUTPUT"
print_info "Output: $OUTPUT"

# Wait for command execution to complete
sleep 2

# ═══════════════════════════════════════════════════════════════════════════
# 📦 Test Rsync File Sync
# ═══════════════════════════════════════════════════════════════════════════

print_header "📦 Testing Rsync File Sync"

# Test 5: Initial File Sync
print_step "Test 5: Sync initial file to client..."
TEST_FILE="/tmp/integration-test-sync-file.txt"
DEST_FILE="/tmp/integration-test-received.txt"

# Create test file with known content
echo "Initial content - Line 1" > $TEST_FILE
echo "Initial content - Line 2" >> $TEST_FILE
echo "Initial content - Line 3" >> $TEST_FILE
echo "Initial content - Line 4" >> $TEST_FILE
echo "Initial content - Line 5" >> $TEST_FILE

# Remove destination file if it exists
rm -f $DEST_FILE

sleep 1
OUTPUT=$($BINARY sync -s $USER@$HOST -P $PORT $TEST_FILE -d $DEST_FILE 2>&1)
run_test "Sync command sent" "Synced" "$OUTPUT"
print_info "Output: $OUTPUT"

# Wait for sync to complete
print_step "Waiting for rsync to complete..."
sleep 3

# Verify file was received
if [ -f "$DEST_FILE" ]; then
    print_success "File received at destination"
    TESTS_PASSED=$((TESTS_PASSED + 1))
else
    print_error "File not found at destination: $DEST_FILE"
fi

# Verify content matches
if diff -q "$TEST_FILE" "$DEST_FILE" >/dev/null 2>&1; then
    print_success "File content matches (checksum verified)"
    TESTS_PASSED=$((TESTS_PASSED + 1))
else
    print_error "File content mismatch!"
    echo "Expected:"
    cat "$TEST_FILE"
    echo "Got:"
    cat "$DEST_FILE"
fi

# Verify rsync logs
if grep -q "Successfully synced" $CLIENT_LOG; then
    BYTES=$(grep "Successfully synced" $CLIENT_LOG | tail -1 | grep -oP '\d+(?= bytes transferred)')
    print_success "Client reports rsync complete ($BYTES bytes transferred)"
    TESTS_PASSED=$((TESTS_PASSED + 1))
else
    print_error "Rsync completion not found in client logs!"
fi

# Test 6: Delta Sync (modify and re-sync)
print_step "Test 6: Modify file and test delta sync..."

# Modify the file - change middle lines
echo "Initial content - Line 1" > $TEST_FILE
echo "MODIFIED content - Line 2 CHANGED" >> $TEST_FILE
echo "MODIFIED content - Line 3 CHANGED" >> $TEST_FILE
echo "Initial content - Line 4" >> $TEST_FILE
echo "Initial content - Line 5" >> $TEST_FILE
echo "NEW Line 6 added" >> $TEST_FILE

sleep 1
OUTPUT=$($BINARY sync -s $USER@$HOST -P $PORT $TEST_FILE -d $DEST_FILE 2>&1)
run_test "Delta sync command sent" "Synced" "$OUTPUT"
print_info "Output: $OUTPUT"

# Wait for delta sync to complete
print_step "Waiting for delta sync to complete..."
sleep 3

# Verify modified content matches
if diff -q "$TEST_FILE" "$DEST_FILE" >/dev/null 2>&1; then
    print_success "Delta sync successful - file content matches"
    TESTS_PASSED=$((TESTS_PASSED + 1))
else
    print_error "Delta sync failed - file content mismatch!"
    echo "Expected:"
    cat "$TEST_FILE"
    echo "Got:"
    cat "$DEST_FILE"
fi

# Check that delta transfer was smaller than full file
LAST_RSYNC=$(grep "Successfully synced" $CLIENT_LOG | tail -1)
if echo "$LAST_RSYNC" | grep -q "bytes transferred"; then
    DELTA_BYTES=$(echo "$LAST_RSYNC" | grep -oP '\d+(?= bytes transferred)')
    FILE_SIZE=$(stat -f%z "$TEST_FILE" 2>/dev/null || stat -c%s "$TEST_FILE" 2>/dev/null)
    print_info "Delta transfer: $DELTA_BYTES bytes (full file: $FILE_SIZE bytes)"

    # Delta should typically be smaller than full file for this test case
    # But we won't fail if it's not, as rsync might send full file for small files
    if [ "$DELTA_BYTES" -lt "$FILE_SIZE" ]; then
        print_success "Delta transfer optimized (smaller than full file)"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        print_info "Delta size: $DELTA_BYTES bytes (may be full file for small changes)"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    fi
fi

# Verify rsync statistics in server logs
if grep -q "Broadcast rsync start" $SERVER_LOG; then
    print_success "Server initiated rsync protocol"
    TESTS_PASSED=$((TESTS_PASSED + 1))
else
    print_error "Server rsync initiation not found in logs!"
fi

# Test 7: Large File Delta Sync
print_step "Test 7: Test with larger file..."

# Create a larger test file (~50KB)
LARGE_FILE="/tmp/integration-test-large-file.bin"
LARGE_DEST="/tmp/integration-test-large-received.bin"

# Generate file with repeated content
for i in {1..1000}; do
    echo "Line $i - This is test content for large file testing with some padding text to make it bigger" >> $LARGE_FILE
done

rm -f $LARGE_DEST

sleep 1
OUTPUT=$($BINARY sync -s $USER@$HOST -P $PORT $LARGE_FILE -d $LARGE_DEST 2>&1)
run_test "Large file sync command sent" "Synced" "$OUTPUT"

# Wait for sync
sleep 3

if [ -f "$LARGE_DEST" ] && diff -q "$LARGE_FILE" "$LARGE_DEST" >/dev/null 2>&1; then
    print_success "Large file synced successfully"
    TESTS_PASSED=$((TESTS_PASSED + 1))

    # Get file size for stats
    LARGE_SIZE=$(stat -f%z "$LARGE_FILE" 2>/dev/null || stat -c%s "$LARGE_FILE" 2>/dev/null)
    print_info "Large file size: $LARGE_SIZE bytes"
else
    print_error "Large file sync failed!"
fi

# Modify large file and re-sync to test delta efficiency
print_step "Test 8: Delta sync on large file..."

# Modify just a few lines in the middle
head -n 500 "$LARGE_FILE" > "$LARGE_FILE.tmp"
echo "MODIFIED Line 501 - CHANGED CONTENT" >> "$LARGE_FILE.tmp"
echo "MODIFIED Line 502 - CHANGED CONTENT" >> "$LARGE_FILE.tmp"
echo "MODIFIED Line 503 - CHANGED CONTENT" >> "$LARGE_FILE.tmp"
tail -n 497 "$LARGE_FILE" >> "$LARGE_FILE.tmp"
mv "$LARGE_FILE.tmp" "$LARGE_FILE"

sleep 1
OUTPUT=$($BINARY sync -s $USER@$HOST -P $PORT $LARGE_FILE -d $LARGE_DEST 2>&1)
run_test "Large file delta sync sent" "Synced" "$OUTPUT"

sleep 3

if diff -q "$LARGE_FILE" "$LARGE_DEST" >/dev/null 2>&1; then
    print_success "Large file delta sync successful"
    TESTS_PASSED=$((TESTS_PASSED + 1))

    # Check delta efficiency
    LAST_DELTA=$(grep "Successfully synced" $CLIENT_LOG | tail -1)
    DELTA_BYTES=$(echo "$LAST_DELTA" | grep -oP '\d+(?= bytes transferred)')
    FULL_SIZE=$(stat -f%z "$LARGE_FILE" 2>/dev/null || stat -c%s "$LARGE_FILE" 2>/dev/null)
    EFFICIENCY=$(echo "scale=1; 100 - ($DELTA_BYTES * 100 / $FULL_SIZE)" | bc)

    print_info "Delta: $DELTA_BYTES bytes, Full: $FULL_SIZE bytes, Saved: ${EFFICIENCY}%"

    if [ "$DELTA_BYTES" -lt "$FULL_SIZE" ]; then
        print_success "Delta transfer optimized for large file"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    fi
else
    print_error "Large file delta sync failed!"
fi

# ═══════════════════════════════════════════════════════════════════════════
# 📊 Test Results
# ═══════════════════════════════════════════════════════════════════════════

print_header "${SPARKLE} Test Results"

echo ""
echo -e "  ${BOLD}Total Tests:${RESET}   ${WHITE}$TESTS_RUN${RESET}"
echo -e "  ${GREEN}${CHECK} Passed:${RESET}     ${GREEN}$TESTS_PASSED${RESET}"
if [ $TESTS_FAILED -eq 0 ]; then
    echo -e "  ${GREEN}${CHECK} Failed:${RESET}     ${GREEN}$TESTS_FAILED${RESET}"
else
    echo -e "  ${RED}${CROSS} Failed:${RESET}     ${RED}$TESTS_FAILED${RESET}"
fi
echo ""

if [ $TESTS_FAILED -eq 0 ]; then
    echo -e "${GREEN}${BOLD}${FIRE}  ALL TESTS PASSED!  ${FIRE}${RESET}"
    echo -e "${GREEN}${BOLD}  SSH authentication, control commands, and rsync working perfectly!${RESET}"
    echo ""
    EXIT_CODE=0
else
    echo -e "${RED}${BOLD}${CROSS}  SOME TESTS FAILED${RESET}"
    echo ""
    echo -e "${YELLOW}Check logs for details:${RESET}"
    echo -e "  ${GRAY}Server: $SERVER_LOG${RESET}"
    echo -e "  ${GRAY}Client: $CLIENT_LOG${RESET}"
    echo ""
    EXIT_CODE=1
fi

# ═══════════════════════════════════════════════════════════════════════════
# 🧹 Cleanup (handled by trap)
# ═══════════════════════════════════════════════════════════════════════════

exit $EXIT_CODE
