Output demo/demo.gif

Set Shell "bash"
Set FontSize 24
Set Width 1600
Set Height 1200
Set Framerate 20
Set Padding 10
Set TypingSpeed 50ms
Set PlaybackSpeed 0.3
Set Theme "Dracula"

Hide
Type "cargo run -- fixtures/sample.usage.kdl" Enter
Sleep 1000ms
Show

# Navigate to "deploy"
Sleep 200ms
Type@100ms "/dly"
Sleep 300ms

# Switch to "Flags"
Tab@100ms
Down@100ms
# Activate checkboxes
Type " "
Sleep 200ms
Down
Sleep 50ms
Type " "
Sleep 100ms

# Switch to "Arguments"
Tab
Sleep 50ms
Enter
Sleep 100ms
Down
Sleep 100ms
# Select "prod"
Type "pr"
Sleep 100ms
Down
Sleep 100ms
Enter
Sleep 100ms
# Change selection
Enter
Sleep 100ms
Up
Sleep 100ms
Enter
Sleep 100ms

# Go back to "Commands"
Tab
Sleep 100ms
Down
Sleep 100ms
Tab
Sleep 100ms

# Navigate to "plugin update"
Down@100ms 5
Sleep 100ms

# Go to "Arguments"
Tab@100ms 2
Sleep 100ms
# Choose a name
Enter
Sleep 100ms
Down@100ms 2
Sleep 100ms
Enter
Sleep 100ms

# Go back to "Commands" and navigate down
Tab
Sleep 100ms
Down
Sleep 100ms
Tab
Sleep 100ms
Down@100ms 2
Sleep 500ms
Type "q"
