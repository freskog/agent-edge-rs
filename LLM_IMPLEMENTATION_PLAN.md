# LLM Implementation Plan - Agent Edge RS

## 🎯 Project Vision
Build a voice assistant that replaces Google Speaker with superior intelligence through:
- **Fast direct actions** (< 500ms): Music control, lights, simple queries
- **Intelligent dialogue flows**: Multi-turn conversations, user questioning, context awareness
- **Smart home integration**: Home Assistant MCP, Spotify control
- **Natural conversation**: Better than basic smart speakers through contextual understanding

---

## 🏗️ Architecture Overview

### LLM-as-Router with Fine-Grained Cancellation
```
Voice Input → user_instruction → process_user_instruction → Audio Output
                ↓                        ↓
            (parallel)           LLM + Tools + TTS
                ↓                        ↓
        New input cancels          (cancellation token)
        current processing             

Happy Path:     Tool → Success(Some(msg)) → TTS (< 500ms)
Processing:     Tool → Success(data) → LLM → formatted response → TTS  
Escalation:     Tool → Escalation(context) → LLM → recovery response → TTS
Silent:         Tool → Success(None) → (no TTS output)
```

### Core Components
```
src/llm/
├── client.rs              (✅ existing Groq client with function calling + cancellation)
├── tools/
│   ├── mod.rs             (tool registry & cancellation-aware execution)
│   ├── spotify.rs         (Spotify/Spotifyd control)
│   ├── home_assistant.rs  (HA MCP integration)
│   ├── dialogue.rs        (ask_user, discuss tools)
│   ├── quick_actions.rs   (time, math, simple queries)
│   └── weather.rs         (weather integration)
├── dialogue/
│   ├── flow_manager.rs    (conversation state machine)
│   ├── context.rs         (enhanced conversation context)
│   └── modes.rs           (SingleTurn, AskingUser, Discussing)
├── response/
│   ├── formatter.rs       (response formatting for voice)
│   └── streaming.rs       (streaming response handling)
└── integration.rs         (process_user_instruction with cancellation)
```

### Cancellation Architecture
```
Main Loop:
┌─────────────────────┐    ┌──────────────────────────┐
│  user_instruction   │    │ process_user_instruction │
│                     │    │                          │
│ Voice → Wake Word   │◄──►│  LLM + Tools + TTS       │
│ → STT → Transcript  │    │  (with CancellationToken) │
└─────────────────────┘    └──────────────────────────┘
         │                           │
         │      New transcript       │
         └──────── cancels ──────────┘
              current processing

Key Principles:
- Fine-grained cancellation (stop immediately anywhere)
- Silent cancellation (no "stopping" messages)
- Empty transcript = pure abort
- All async operations respect CancellationToken
```

---

## 📅 Implementation Timeline

### **Week 1: Foundation & Cancellable Time Tool**
**Goal**: Complete end-to-end LLM tool system with cancellation support

#### Deliverables:
- [ ] Tool registry with cancellation support (`tools/mod.rs`)
- [ ] Simple time tool with routing parameter (`tools/quick_actions.rs`)
- [ ] Enhanced LLM client with function calling + cancellation
- [ ] `process_user_instruction` with fine-grained cancellation
- [ ] Integration with main voice loop (parallel processing)
- [ ] Complete unit and integration tests

#### Success Criteria:
- "What time is it" → LLM calls get_time(send_output_directly_to_tts: true) → Success(Some("It's 3:45 PM")) → TTS (< 500ms)
- "What time will it be in 2 hours" → LLM calls get_time(send_output_directly_to_tts: false) → Success(Some("3:45 PM")) → LLM calculates → "It will be 5:45 PM"
- Interruption: New transcript cancels current processing immediately
- Empty transcript: Pure abort with no error messages
- Error handling: get_time() fails → Escalation(error_context) → LLM explains gracefully

#### Technical Tasks:
```rust
// New structs and enums with cancellation
enum ToolResult { Success(Option<String>), Escalation(Value) }
struct Tool { name: String, description: String, parameters: Value /* includes send_output_directly_to_tts */ }

// Universal tool parameter
send_output_directly_to_tts: boolean  // Added to every tool

// Key functions to implement
async fn process_user_instruction(transcript: &str, cancel: CancellationToken) -> Result<()>
async fn execute_tool_with_cancellation(tool_call: ToolCall, cancel: CancellationToken) -> Result<ToolResult>
async fn main_loop_with_parallel_processing() -> Result<()>
```

### **Week 2: Dialogue Foundation**
**Goal**: ask_user tool and simple dialogue flows

#### Deliverables:
- [ ] Conversation state machine (`dialogue/flow_manager.rs`)
- [ ] ask_user tool implementation (`tools/dialogue.rs`)
- [ ] Enhanced conversation context with modes
- [ ] User input timeout handling
- [ ] Integration with main voice loop

#### Success Criteria:
- LLM can ask follow-up questions and wait for responses
- "Set temperature" → "What temperature?" → "72" → thermostat set
- Conversation timeouts handled gracefully

#### Technical Tasks:
```rust
enum ConversationMode {
    SingleTurn,
    AskingUser { question_id: String, timeout: Instant },
    Discussing { topic: String, goal: String, turn_count: u32 }
}

async fn ask_user(question: &str) -> Result<String>
async fn handle_user_response(response: &str, context: &ConversationMode) -> Result<Action>
```

### **Week 3: Home Assistant Integration**
**Goal**: MCP integration with Home Assistant

#### Deliverables:
- [ ] Home Assistant MCP tool (`tools/home_assistant.rs`)
- [ ] MCP client implementation
- [ ] HA entity discovery and control
- [ ] Smart query interpretation (LLM-processed responses)
- [ ] Error handling for HA unavailability

#### Success Criteria:
- "Turn on living room lights" → lights turn on
- "What's my energy usage?" → interpreted data response
- "Make it cooler" → thermostat adjustment

#### Technical Tasks:
```rust
struct HomeAssistantTool {
    mcp_client: MCPClient,
    entity_cache: HashMap<String, Entity>,
}

async fn query_home_assistant(query: &str) -> Result<Value>
async fn control_device(entity_id: &str, action: &str, value: Option<Value>) -> Result<String>
async fn discover_entities() -> Result<Vec<Entity>>
```

### **Week 4: Advanced Dialogue**
**Goal**: discuss tool and multi-turn conversations

#### Deliverables:
- [ ] discuss tool implementation
- [ ] Multi-turn conversation management
- [ ] Context preservation across turns
- [ ] Conversation summarization
- [ ] Goal tracking and completion

#### Success Criteria:
- "Plan my evening" → multi-turn planning conversation
- Context maintained across 5+ exchanges
- Natural conversation flow with goal completion

#### Technical Tasks:
```rust
async fn discuss(topic: &str, goal: &str) -> Result<DiscussionState>
async fn continue_discussion(user_input: &str, state: &mut DiscussionState) -> Result<Response>
struct DiscussionState { topic: String, goal: String, context: Vec<Message>, turn_count: u32 }
```

### **Week 5: Optimization & Polish**
**Goal**: Performance tuning and production readiness

#### Deliverables:
- [ ] Response caching system
- [ ] Parallel tool execution
- [ ] Streaming response optimization
- [ ] Error recovery and fallback strategies
- [ ] Performance metrics and monitoring

#### Success Criteria:
- 95% of fast commands under 500ms
- Dialogue flows feel natural and responsive
- Graceful degradation when services unavailable
- Production-ready error handling

---

## 🔧 Technical Specifications

### Tool Definitions with Universal Parameter

#### Standard Tool Signature
Every tool includes the universal routing parameter:
```rust
tool_name(params..., send_output_directly_to_tts: boolean)
```

#### Tool Examples with Routing Logic

```rust
// Time Tool with Routing
get_time(send_output_directly_to_tts: boolean)
// Direct output: Success(Some("It's 3:45 PM"))
// For processing: Success(Some("3:45 PM"))  // Raw data for LLM
// Error: Escalation({"error": "system_clock_unavailable"})

// Spotify Control with Silent/Vocal Options
spotify_control(action: "play"|"pause"|"next"|"previous"|"volume", value?: number, send_output_directly_to_tts: boolean)
// Silent success: Success(None)  // Volume changed quietly
// Vocal success: Success(Some("Music paused"))
// Error escalation: Escalation({"error": "spotify_unavailable", "action": "pause"})
// Empty queue: Escalation({"error": "empty_queue", "action": "next"})

// Home Assistant Controls
ha_control(entity: String, action: String, value?: Value, send_output_directly_to_tts: boolean)  
// Silent success: Success(None)  // Lights turned on quietly
// Vocal success: Success(Some("Living room lights turned on"))
// Error: Escalation({"error": "device_offline", "entity": "living_room_lights"})

// Complex Queries (Always Escalate)
weather(location?: String, send_output_directly_to_tts: boolean)
// Always returns: Escalation(weather_data) → LLM formats naturally

ha_query(query: String, send_output_directly_to_tts: boolean) 
// Always returns: Escalation(complex_data) → LLM interprets and explains

spotify_search(query: String, send_output_directly_to_tts: boolean)
// Always returns: Escalation(search_results) → LLM helps user choose
```

#### LLM Routing Examples
```rust
User: "What time is it?"
→ LLM calls: get_time(send_output_directly_to_tts: true)
→ Tool returns: Success(Some("It's 3:45 PM"))  
→ TTS: "It's 3:45 PM" (direct output)

User: "What time will it be in 2 hours?"  
→ LLM calls: get_time(send_output_directly_to_tts: false)
→ Tool returns: Success(Some("3:45 PM"))
→ LLM receives raw data → calculates → TTS: "It will be 5:45 PM"

User: "Turn up the volume"
→ LLM calls: spotify_control("volume_up", send_output_directly_to_tts: true)
→ Tool returns: Success(None)  // Silent volume change
→ No TTS output

User: "Next song"
→ LLM calls: spotify_control("next", send_output_directly_to_tts: true)  
→ Tool returns: Escalation({"error": "empty_queue"})
→ LLM processes: "Your queue is empty. Would you like me to find some music?"
```

### Escalation Pattern Examples

#### Happy Path Scenarios (Direct Results)
```rust
User: "Pause music"
→ LLM calls spotify_control("pause") 
→ Tool returns Direct("Music paused")
→ TTS: "Music paused" (< 500ms total)

User: "Turn on the lights"  
→ LLM calls ha_control("living_room_lights", "on")
→ Tool returns Direct("Living room lights turned on")
→ TTS: "Living room lights turned on" (< 500ms total)
```

#### Escalation Scenarios (Error Handling)
```rust
User: "Next song"
→ LLM calls spotify_control("next")
→ Tool returns NeedsProcessing({"error": "empty_queue", "context": "no_songs_remaining"})
→ LLM processes: "It looks like your queue is empty. Would you like me to find some music?"

User: "Make it warmer"
→ LLM calls ha_control("thermostat", "increase") 
→ Tool returns NeedsProcessing({"error": "device_offline", "last_temp": "68F"})
→ LLM processes: "I can't reach your thermostat right now. The last temperature was 68 degrees."
```

#### Complex Data Scenarios (Always Escalate)
```rust
User: "What's the weather like?"
→ LLM calls weather("current_location")
→ Tool returns NeedsProcessing({"temp": 72, "condition": "partly_cloudy", "wind": "5mph"})
→ LLM processes: "It's 72 degrees with partly cloudy skies and light winds around 5 miles per hour."

User: "How's my energy usage?"
→ LLM calls ha_query("energy_usage_today")  
→ Tool returns NeedsProcessing({"usage_kwh": 15.2, "vs_yesterday": "+12%", "cost": "$1.83"})
→ LLM processes: "You've used 15.2 kilowatt hours today, which is 12% more than yesterday, costing about $1.83."
```

#### Sub-flow Recovery Scenarios  
```rust
User: "Plan a relaxing evening"
→ LLM calls discuss("evening", "relaxation")
→ Tool tries: start_jazz_playlist() + dim_lights()
→ Music succeeds, lights fail
→ Tool returns NeedsProcessing({"completed": ["music_started"], "failed": ["lights_offline"], "suggestions": ["candles", "manual_dimmer"]})
→ LLM processes: "I've started some relaxing jazz music. Your smart lights seem to be offline, but you could try dimming them manually or lighting some candles for ambiance."
```

### System Prompt for Tool Routing

```
You are a voice assistant. When calling tools, always include the 'send_output_directly_to_tts' parameter:

- true: Send the tool's output directly to speech (for final answers to the user)
- false: Return the tool result to you for further processing/calculation

Guidelines:
- Use true when the user wants a direct answer: "What time is it?" → get_time(send_output_directly_to_tts: true)
- Use false when you need to process the result: "What time will it be in 2 hours?" → get_time(send_output_directly_to_tts: false)
- Use true for simple commands: "Turn up volume" → spotify_control("volume_up", send_output_directly_to_tts: true)
- Use false for multi-step tasks: "Play jazz and lower volume" → need multiple tool calls with processing

Tool responses with send_output_directly_to_tts: true should be natural speech-ready text.
Tool responses with send_output_directly_to_tts: false return raw data for your processing.

Remember: Empty transcript means user interrupted - return immediately without explanation.
```

### Configuration Structure
```rust
struct LLMConfig {
    // Tool execution timeouts
    tool_execution_timeout: Duration,    // 5s (max time for any tool to execute)
    escalation_processing_timeout: Duration, // 10s (LLM processing of complex results)
    dialogue_timeout: Duration,          // 30s (waiting for user response)
    
    // Conversation limits
    max_discussion_turns: u32,           // 10
    max_context_messages: usize,         // 20
    max_escalation_retries: u32,         // 3 (retries for failed tool execution)
    
    // Service endpoints
    spotifyd_endpoint: String,
    home_assistant_url: String,
    home_assistant_token: SecretBox<String>,
    
    // LLM settings
    model: String,                       // "llama-3.3-70b-versatile"
    temperature: f32,                    // 0.7 for dialogue, 0.3 for tools
    max_tokens: u32,                     // 4096
}
```

### Response Flow Architecture

#### Main Loop with Parallel Processing
```rust
async fn main() -> Result<()> {
    let mut current_processing: Option<(JoinHandle<()>, CancellationToken)> = None;

    loop {
        tokio::select! {
            Ok(instruction) = detector.get_instruction() => {
                // Cancel any current processing immediately
                if let Some((handle, cancel_token)) = current_processing.take() {
                    cancel_token.cancel();
                    handle.abort(); // Don't wait for graceful shutdown
                }

                // Start new processing (even empty transcript is valid)
                let cancel_token = CancellationToken::new();
                let handle = tokio::spawn(process_user_instruction(instruction.text, cancel_token.clone()));
                current_processing = Some((handle, cancel_token));
            }
            
            Some(result) = async { /* current processing completion */ } => {
                current_processing = None;
                // Handle result if needed
            }
        }
    }
}
```

#### process_user_instruction with Cancellation
```rust
async fn process_user_instruction(transcript: &str, cancel: CancellationToken) -> Result<()> {
    // Empty transcript = pure abort
    if transcript.trim().is_empty() {
        return Ok(());
    }

    // Add to context and get LLM response
    context.add_user_message(transcript);
    let response = llm.complete_with_tools_and_cancellation(
        context.get_messages(), 
        all_tools(), 
        cancel.clone()
    ).await?;

    // Process tool calls with routing logic
    for tool_call in response.tool_calls {
        let send_directly = tool_call.arguments["send_output_directly_to_tts"]
            .as_bool().unwrap_or(false);
            
        let result = execute_tool_with_cancellation(tool_call, cancel.clone()).await?;
        
        return match (send_directly, result) {
            (true, ToolResult::Success(Some(message))) => {
                // Direct speech output
                tts.speak_with_cancellation(message, cancel).await?;
                Ok(())
            },
            (true, ToolResult::Success(None)) => {
                // Silent success, no TTS
                Ok(())
            },
            (false, ToolResult::Success(data)) => {
                // Return to LLM for processing
                let formatted = llm.process_tool_result_with_cancellation(tool_call, data, cancel).await?;
                tts.speak_with_cancellation(formatted, cancel).await?;
                Ok(())
            },
            (_, ToolResult::Escalation(context)) => {
                // Always escalate to LLM regardless of routing
                let recovery = llm.handle_escalation_with_cancellation(tool_call, context, cancel).await?;
                tts.speak_with_cancellation(recovery, cancel).await?;
                Ok(())
            }
        };
    }

    // No tools called - speak LLM response directly
    if !response.content.is_empty() {
        tts.speak_with_cancellation(response.content, cancel).await?;
    }
    
    Ok(())
}
```

---

## 🧪 Testing Strategy

### Unit Tests
- [ ] Individual tool execution with happy/error paths
- [ ] Dynamic tool result processing (Direct vs NeedsProcessing)
- [ ] Conversation state transitions and side effects
- [ ] LLM tool selection accuracy
- [ ] Escalation context handling and formatting

### Integration Tests  
- [ ] Spotify integration with mock spotifyd (happy + failure scenarios)
- [ ] Home Assistant MCP integration with mock server
- [ ] End-to-end dialogue flows with escalation
- [ ] Tool execution timeouts and error escalation
- [ ] Complex multi-tool scenarios with partial failures

### Performance Tests
- [ ] Fast tool latency benchmarks (target: < 500ms)
- [ ] Dialogue flow responsiveness
- [ ] Memory usage during long conversations
- [ ] Concurrent request handling

### Voice Tests
- [ ] Voice command recognition accuracy
- [ ] TTS integration with tool responses
- [ ] Audio feedback during tool execution
- [ ] Interruption handling during long operations

---

## 📊 Success Metrics

### Performance Targets
- **Direct Tool Commands**: 95% under 500ms end-to-end (LLM + tool execution + TTS)
- **Tool Selection Accuracy**: LLM picks correct tool 95% of the time
- **Tool Reliability**: 99% success rate for available services
- **Dialogue Quality**: Natural conversation flow with < 2s response time
- **Context Retention**: Maintain context across 10+ turn conversations

### Feature Completeness
- [ ] Replace basic Google Speaker functionality
- [ ] Superior contextual understanding
- [ ] Reliable smart home control
- [ ] Natural multi-turn conversations
- [ ] Graceful error handling and recovery

### User Experience
- [ ] Voice commands feel instant for simple actions
- [ ] Conversations feel natural and intelligent
- [ ] System provides helpful feedback during longer operations
- [ ] Error messages are clear and actionable

---

## 🚀 Post-MVP Enhancements

### Advanced Features (Future Phases)
- [ ] Learning user preferences and patterns
- [ ] Proactive suggestions and automation
- [ ] Multi-room audio coordination
- [ ] Calendar and scheduling integration
- [ ] Advanced home automation scenarios

### Technical Improvements
- [ ] Local LLM option for privacy
- [ ] Voice training and personalization
- [ ] Advanced audio processing
- [ ] Mobile app companion
- [ ] Web dashboard for configuration

---

## 📝 Weekly Review Template

### Week X Review
**Completed:**
- [ ] Task 1
- [ ] Task 2

**Challenges:**
- Issue description and resolution

**Next Week Focus:**
- Priority tasks for upcoming week

**Performance Updates:**
- Latency measurements
- Success rates
- User feedback

**Technical Debt:**
- Items to address
- Refactoring needs

---

*Last Updated: [Date]*
*Version: 1.0* 