# Use an official Python base image
FROM python:3.10-slim

# Set work directory
WORKDIR /app

# Install system dependencies including curl for health checks
RUN apt-get update && apt-get install -y curl && rm -rf /var/lib/apt/lists/*

# Copy only requirements first to leverage Docker cache
COPY pyproject.toml ./

# Install pip and dependencies
RUN pip install --upgrade pip && \
    pip install httpx>=0.28.1 fastmcp uvicorn

# Copy project files
COPY . .

# Expose port 8000 which is the default port for SSE transport in MCP SDK
EXPOSE 8000

# Default command - just run the script (which now uses SSE transport by default)
CMD ["python", "weather.py"]