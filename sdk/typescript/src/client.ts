import {Transport} from './transport.js';

/** Shared connection ownership for the public handles. */
export abstract class Client {
  #closed = false;
  protected constructor(private readonly connection: Transport, private readonly ownsConnection = true) {}

  protected get transport(): Transport {
    if (this.#closed) throw new Error('SDK client is closed');
    return this.connection;
  }

  close(): void {
    this.#closed = true;
    if (this.ownsConnection) this.connection.close();
  }
}
