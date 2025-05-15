# Use an official Python base image
FROM python:3.10-slim

# Set work directory
WORKDIR /app

# Copy only requirements first to leverage Docker cache
COPY pyproject.toml ./

# Install pip and dependencies
RUN pip install --upgrade pip && \
    pip install httpx>=0.28.1 mcp[cli]>=1.2.0

# Copy project files
COPY . .

# Expose port 8000 which is the default port for SSE transport in MCP SDK
EXPOSE 8000

# Default command - just run the script (which now uses SSE transport by default)
CMD ["python", "weather.py"]