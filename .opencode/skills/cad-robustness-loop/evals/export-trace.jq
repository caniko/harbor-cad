# Export only visible assistant text and actual tool calls/results. Exclude
# reasoning, system messages and opaque native/provider metadata.
if (.data | type) == "object" and (.data.messages | type) == "array"
then {data: .data.messages, history_source: "full_session_export"}
else . + {history_source: "active_context"} end |
{
  session_id: env.SESSION_ID,
  history_source,
  has_completed_answer: any(.data[]; .type == "assistant" and .finish == "stop"),
  messages: [.data[] | select(.type == "assistant" or .type == "user") |
    if .type == "user" then {
      id, time, type,
      content: [{type: "text", text: .text}]
    } else {
    id, time, type, model, finish, tokens,
    content: [.content[] |
      if .type == "text" then {type, text}
      elif .type == "tool" then {
        type, name, time, executed, status: .state.status, input: .state.input,
        result: [.state.content[]? | select(.type == "text") | {type, text}]
      } else empty end]
  } end]
}
