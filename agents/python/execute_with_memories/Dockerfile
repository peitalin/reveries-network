FROM python:3.10-slim

WORKDIR /app

# Copy Python requirements and install dependencies
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Copy the Python application
COPY . .

# Expose the port the Python server listens on
EXPOSE 8000

# Start the Python server
CMD ["python", "main.py"]