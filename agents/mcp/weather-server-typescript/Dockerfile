# Use an official Node.js base image
FROM node:20-slim

# Set work directory
WORKDIR /app

# Copy package.json and package-lock.json (if present)
COPY package*.json ./

# Install dependencies
RUN npm install --production

# Copy the rest of the project files
COPY . .

# Build the TypeScript project (if using TypeScript)
RUN if [ -f tsconfig.json ]; then npm run build; fi

# Expose the port (default 5002, can be overridden)
EXPOSE 5002

# Default command (adjust if entrypoint is different)
CMD ["node", "dist/weather.js", "--transport", "http", "--host", "0.0.0.0", "--port", "5002"]