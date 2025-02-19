export class JsonRpcWebSocket {
  private ws: WebSocket;
  private subscriptionId: string | null = null;
  private messageHandler: ((data: any) => void) | null = null;

  constructor(url: string) {
    this.ws = new WebSocket(url);
    this.setupEventListeners();
  }

  private setupEventListeners() {
    this.ws.onmessage = (event) => {
      const data = JSON.parse(event.data);
      if (data.params?.subscription && this.messageHandler) {
        this.messageHandler(data.params.result);
      }
    };
  }

  public async subscribe(
    method: string,
    params: any[],
    handler: (data: any) => void
  ): Promise<void> {
    return new Promise((resolve, reject) => {
      this.messageHandler = handler;

      this.ws.onopen = () => {
        // Send subscription request
        this.ws.send(JSON.stringify({
          jsonrpc: '2.0',
          id: 1,
          method: method,
          params: params
        }));
        resolve();
      };

      this.ws.onerror = (error) => {
        reject(error);
      };
    });
  }

  public unsubscribe() {
    if (this.subscriptionId) {
      this.ws.send(JSON.stringify({
        jsonrpc: '2.0',
        id: 2,
        method: 'unsubscribe_hb',
        params: [this.subscriptionId]
      }));
    }
    this.ws.close();
  }
}