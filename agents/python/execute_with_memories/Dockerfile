FROM python:3.10-slim

WORKDIR /app

# Copy Python requirements and install dependencies
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Copy the Python application
COPY . .

# Add execute permissions to the MCP script
RUN chmod +x /app/weather_mcp_local/weather_mcp.py

# Expose the port the Python server listens on
EXPOSE 6000

# Start the Python server
CMD ["python", "main.py"]