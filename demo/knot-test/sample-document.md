# Sample Document for Knot Processing

## Introduction

This is a sample document that demonstrates the Knot file
processing pipeline. When placed in the watched source
directory, Knot should automatically process it using the
configured agent.

## Key Points

1. The Knot service watches a source directory for file events.
2. When a file is created or modified, the configured agent
   processes its content.
3. The agent output (tie-off) is written to the output directory.
4. Processing events are recorded in the loom-log file.

## Recommendations

- Keep documents concise for faster processing.
- Use markdown format for best results.
- Monitor the loom-log for processing status.
