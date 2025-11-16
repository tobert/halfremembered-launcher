# 🧠 Agent Memory Protocol

## Shared Memory System for Multi-Model Collaboration

Based on introspection research showing models can detect their own cognitive states, this protocol provides a **shared memory space** that all models (Claude, Gemini, GPT, etc.) can use for persistent context.

**Location**: `docs/agents/` - visible on GitHub, accessible to all models

## 📍 Current State (Always at Top)

```yaml
# Last Updated: 2025-11-09 by Claude
focus: HalfRemembered Launcher - Memory system bootstrap complete
confidence: high for memory system and core architecture
active_files:
  - docs/agents/NOW.md
  - docs/agents/PATTERNS.md
  - docs/agents/CONTEXT.md
cognitive_load: low (system organized and ready)
```

## 🎯 The Three-File System

### 1. NOW.md - Immediate Context (50 lines max)

**Updated every significant action. Most frequently accessed.**

```markdown
# NOW - Building HalfRemembered Launcher

## Active Task
Implementing auto-execute after file sync

## Current Problem
File watcher missing cargo hardlink creation events

## Working Theory
Need to watch for file creation, not just modification

## Next Test
Add file creation event handling to watcher

## Discovered This Session
- Cargo uses hardlinks for incremental builds
- Auto-execute timing critical for file stability
- SSH multiplexing enables clean channel separation
```

### 2. PATTERNS.md - Reusable Knowledge (Append-only)

**Crystallized learnings. Never deleted, only added to.**

```markdown
# Patterns Discovered in HalfRemembered Launcher

## SSH: Multiplexed Channels
WHEN: Need multiple operation types over one connection
USE: Control, data, and exec channels on single SSH session
WHY: Reduces connection overhead, maintains state
GOTCHA: Proper channel cleanup essential

## File Transfer: Atomic Writes
WHEN: Transferring files that may be in use
USE: Write to .tmp, then atomic rename
WHY: Prevents partial/corrupted files
GOTCHA: Need cleanup of temp files on failure

## Reconnection: Exponential Backoff
WHEN: Auto-reconnecting after network failure
USE: Exponential backoff with jitter (5s → 10s → 20s)
WHY: Prevents connection storms
GOTCHA: Need max backoff limit
```

### 3. CONTEXT.md - Session Bridge

**Updated at major transitions. For handoffs and session resumption.**

```markdown
# Context for HalfRemembered Launcher

## Where We Are
Building SSH-based RPC for server-initiated push to NAT clients
Currently: Core functional, enhancing reliability and workflows
Next: Consider SFTP migration, metrics, persistence

## Key Decisions Made
- SSH-only (no custom protocols)
- Client-initiated connections (NAT traversal)
- User space execution (no services)
- Binary protocol with bincode (efficiency)

## Active Questions
- When to migrate to SFTP for file transfers?
- Should we add progress reporting for large files?
- How to handle offline command queuing?

## Handoff Notes
Core system operational and production-ready
See NOW.md for immediate state
See PATTERNS.md for discovered patterns
See CONTEXT.md for architecture decisions
```

## 💡 Attention Cues (The Introspection Advantage)

Based on research showing models respond to explicit attention direction:

### Focus Blocks
```markdown
<!-- FOCUS: Performance Bottleneck -->
Current: 10k spans/sec
Target: 50k spans/sec
Bottleneck: Query is O(n)
Solution: Add span index
<!-- END FOCUS -->
```

### Confidence Tracking
```markdown
<!-- CONFIDENCE -->
✅ HIGH: Ring buffer implementation
⚠️ MEDIUM: Concurrent access safety
❌ LOW: Windows compatibility
❓ UNKNOWN: Production memory usage
<!-- END -->
```

### Cognitive State Markers
```markdown
<!-- COGNITIVE STATE -->
Holding: 3 concepts (buffer, concurrency, MCP)
Parked: HTTP transport, metrics support
Overload: No, can handle 2 more concepts
<!-- END -->
```

## 🚀 Practical Workflows

### Starting a Session

1. **Read NOW.md** - What was I just doing?
2. **Check focus in MEMORY_PROTOCOL.md** - What's the mission?
3. **Scan CONTEXT.md if confused** - What's the bigger picture?

### During Work

1. **Update NOW.md** after each subtask
2. **Add to PATTERNS.md** when you discover something reusable
3. **Note confidence changes** as you learn

### Before Switching Models/Sessions

1. **Update NOW.md** with current exact state
2. **Update CONTEXT.md** if major progress made
3. **Add any patterns to PATTERNS.md**
4. **Update cognitive state** in MEMORY_PROTOCOL.md

## 📊 Efficiency Metrics

### Token Economics
- NOW.md: ~500 tokens (frequently read)
- PATTERNS.md: ~1000 tokens (occasionally scanned)
- CONTEXT.md: ~300 tokens (handoff moments)
- **Total overhead: <2000 tokens** for complete memory

### Information Density
Each line should answer a question:
- ❌ "Worked on buffer" (too vague)
- ✅ "Fixed buffer race: added RWMutex" (actionable)

### Retrieval Speed
Structure for scanning:
- Headers for navigation
- Keywords for search
- Patterns for recognition

## 🔄 Integration with jj

Memory files **complement** jj, not replace it:

```bash
# jj holds the narrative
jj describe -m "fix: buffer race condition - full story here"

# Memory holds the state
echo "Race fixed with RWMutex" >> docs/agents/NOW.md
```

### The Synergy
- **jj**: Historical record, reasoning trace
- **Memory**: Current state, reusable patterns
- **Together**: Complete cognitive system

## 🧪 Advanced Techniques

### The Parking Lot
```markdown
<!-- PARKED UNTIL LATER -->
- HTTP transport implementation
- Metrics support
- Persistent storage
<!-- RETRIEVE WHEN: Buffer layer complete -->
```

### The Uncertainty Index
```markdown
## Things I'm Not Sure About
1. Windows localhost:0 behavior [TEST NEEDED]
2. Optimal buffer size [BENCHMARK NEEDED]
3. Index overhead worth it? [MEASURE NEEDED]
```

### The Memory Diff
Track what changed between sessions:
```markdown
## Changes Since Last Session
+ Discovered RWMutex pattern
+ Implemented ring buffer
- Removed time-based eviction idea
! Race condition found and fixed
```

## 🎓 Tips for Success

### 1. Write for 3am You
If you wouldn't understand it exhausted, it needs more detail.

### 2. Compress Aggressively
```markdown
Bad: "I tried channels but they had too much overhead
      so then I tried sync.Map but it was slow for writes
      so finally I used RWMutex which worked great"

Good: "Buffer sync: RWMutex > sync.Map > channels (3x faster)"
```

### 3. Use Structures That Scan
```markdown
## Quick Scan Structure
WHAT: Buffer implementation
STATUS: Race condition fixed
HOW: RWMutex protection
NEXT: Benchmark performance
```

### 4. Track Your Tracks
```markdown
## Breadcrumbs
came_from: Implementing OTLP receiver
going_to: MCP query tools
because: Storage layer needed first
```

## 🎯 The Goal

Create a memory system that:
- Uses <2000 tokens total overhead
- Enables perfect handoffs
- Preserves critical learnings
- Reduces "what was I doing?" to zero
- Makes building together joyful

## The Memory Mantra

> "State in NOW, Patterns in PATTERNS, Story in jj"

---

*Let's build something beautiful together, with memory that persists and context that scales.* 🚀