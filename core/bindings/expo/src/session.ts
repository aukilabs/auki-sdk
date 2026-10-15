import module from "./AukiSdkExpoModule";
import type { AukiServiceEnvironment, ZitadelSessionCredentials } from "./AukiSdkExpo.types";
import { data, domains, type AukiDomainData, type AukiDomains } from "./DomainData";
import { normalizeAukiError } from "./errors";
import { fleet, type AukiFleet } from "./Fleet";
import { jobs, type AukiJobs } from "./Jobs";
import { Peer, type PeerStartOptions } from "./peer";
import { closeSession, importZitadelSession } from "./ZitadelSession";

export type LoginOptions = {
  email: string;
  password: string;
  clientId?: string | null;
  environment?: AukiServiceEnvironment;
};

/** Handle around the session id the app already stores. */
export class Session {
  /** @internal Use Session.import, Session.login, or Session.from. */
  private constructor(readonly id: string) {}

  static from(id: string): Session {
    return new Session(id);
  }

  static async login(options: LoginOptions): Promise<Session> {
    try {
      const id = options.environment
        ? await module.loginWithEnvironment(
          options.environment.apiBaseUrl,
          options.environment.ddsBaseUrl,
          options.environment.dmsBaseUrl,
          options.email,
          options.password,
          options.clientId,
        )
        : await module.loginDev(options.email, options.password, options.clientId);
      return new Session(id);
    } catch (error) {
      throw normalizeAukiError(error, "Sign-in failed");
    }
  }

  static async import(
    credentials: ZitadelSessionCredentials,
    store: Parameters<typeof importZitadelSession>[1],
    environment?: AukiServiceEnvironment,
  ): Promise<Session> {
    try {
      return new Session(await importZitadelSession(credentials, store, environment));
    } catch (error) {
      throw normalizeAukiError(error, "Session import failed");
    }
  }

  get domains(): AukiDomains {
    return domains(this.id);
  }

  domainData(domainId: string): Promise<AukiDomainData> {
    return data(this.id, domainId);
  }

  fleet(domainId: string): Promise<AukiFleet> {
    return fleet(this.id, domainId);
  }

  jobs(domainId: string): Promise<AukiJobs> {
    return jobs(this.id, domainId);
  }

  startPeer(domainId: string, options?: PeerStartOptions): Promise<Peer> {
    return Peer.start(this.id, domainId, options);
  }

  async close(): Promise<void> {
    await closeSession(this.id);
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }
}
