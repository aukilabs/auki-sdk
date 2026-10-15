import module from "./AukiSdkExpoModule";

/** Robot model solved from URDF XML. Web rejects these calls. */
export class UrdfModel {
  private constructor(
    private readonly handle: string,
    private readonly count: number,
  ) {}

  static async fromXml(xml: string): Promise<UrdfModel> {
    const handle = await module.urdfModelFromXml(xml);
    const count = await module.urdfJointCount(handle);
    return new UrdfModel(handle, count);
  }

  jointCount(): number {
    return this.count;
  }

  async resolve(angles: ArrayLike<number>): Promise<unknown> {
    return JSON.parse(await module.urdfResolve(this.handle, Array.from(angles)));
  }

  async resolveIdentity(): Promise<unknown> {
    return JSON.parse(await module.urdfResolveIdentity(this.handle));
  }

  dispose(): void {
    void module.urdfModelFree(this.handle);
  }
}
