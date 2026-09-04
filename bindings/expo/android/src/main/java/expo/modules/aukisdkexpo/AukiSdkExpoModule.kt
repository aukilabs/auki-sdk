package expo.modules.aukisdkexpo

import expo.modules.kotlin.exception.CodedException
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

private class AndroidUnsupportedException :
  CodedException("AukiSdkExpo is not implemented on Android in this slice")

class AukiSdkExpoModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("AukiSdkExpo")

    AsyncFunction("loginDev") { _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("loginWithDomainAccessToken") { _: String, _: String, _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("accessibleDomains") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("startPeer") { _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("startPeerWithDiscovery") { _: String, _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("peerId") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("domainId") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("discover") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("discoverProtocol") { _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("infoFetchExact") { _: String, _: Map<String, String> ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("catalogFetchResourcesExact") {
        _: String,
        _: Map<String, String>,
        _: List<String>,
      ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("registryListExact") { _: String, _: Map<String, String>, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("registryFetchExact") {
        _: String,
        _: Map<String, String>,
        _: String,
        _: String,
        _: String,
      ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("blobFetchExact") { _: String, _: Map<String, String>, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("streamSubscribeExact") {
        _: String,
        _: Map<String, String>,
        _: String,
        _: String,
      ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("streamNext") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("streamCancel") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("urdfModelFromXml") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("urdfJointCount") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("urdfResolve") { _: String, _: List<Double> ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("urdfResolveIdentity") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("urdfModelFree") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("shutdown") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("waitStopped") { _: String ->
      throw AndroidUnsupportedException()
    }
  }
}
