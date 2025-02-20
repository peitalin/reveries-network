export class JsonRpcWebSocket {
  private ws: WebSocket;
  private subscriptionId: string | null = null;
  private messageHandler: ((data: any) => void) | null = null;
  private onCloseCallback?: () => void;

  constructor(url: string) {
    console.log(`Creating WebSocket connection to ${url}`);
    this.ws = new WebSocket(url);
    this.setupEventListeners();
    this.ws.addEventListener('close', () => {
      console.log('WebSocket closed');
      this.onCloseCallback?.();
    });
  }

  private setupEventListeners() {
    this.ws.onmessage = (event) => {
      const data = JSON.parse(event.data);
      if (data.params?.subscription && this.messageHandler) {
        this.messageHandler(data.params.result);
      }
    };

    this.ws.onclose = () => {
      console.log('WebSocket connection closed');
      this.onCloseCallback?.();
    };
  }

  public async subscribe(
    method: string,
    params: any[],
    handler: (data: any) => void
  ): Promise<void> {
    return new Promise((resolve, reject) => {
      this.messageHandler = handler;

      const setupConnection = async () => {
        try {
          console.log('WebSocket opened, sending subscription');
          this.ws.send(JSON.stringify({
            jsonrpc: '2.0',
            id: 1,
            method: method,
            params: params
          }));
          resolve();
        } catch (error) {
          reject(error);
        }
      };

      if (this.ws.readyState === WebSocket.OPEN) {
        setupConnection();
      } else {
        this.ws.onopen = () => setupConnection();
        this.ws.onerror = (error) => {
          console.error('WebSocket error:', error);
          reject(error);
        };
      }
    });
  }

  public async unsubscribe() {
    if (this.subscriptionId && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify({
        jsonrpc: '2.0',
        id: 2,
        method: 'unsubscribe_hb',
        params: [this.subscriptionId]
      }));
    }
    if (this.ws.readyState !== WebSocket.CLOSED) {
      this.ws.close();
    }
  }

  onClose(callback: () => void) {
    this.onCloseCallback = callback;
  }

  public async isConnected(): Promise<boolean> {
    return this.ws.readyState === WebSocket.OPEN;
  }
}