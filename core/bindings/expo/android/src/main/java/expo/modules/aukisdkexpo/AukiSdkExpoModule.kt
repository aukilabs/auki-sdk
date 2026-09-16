package expo.modules.aukisdkexpo

import expo.modules.kotlin.exception.CodedException
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

private class AndroidUnsupportedException :
  CodedException("AukiSdkExpo is not implemented on Android in this slice")

class AukiSdkExpoModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("AukiSdkExpo")

    AsyncFunction("loginDev") { _: String, _: String, _: String? ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("loginWithEnvironment") {
        _: String,
        _: String,
        _: String,
        _: String,
        _: String,
        _: String?,
      ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("accessibleDomains") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("domainsList") { _: String, _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("domainsForPortal") { _: String, _: String, _: String?, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("domainsPortals") { _: String, _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("domainsPortal") { _: String, _: String, _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("domainDataOpen") { _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("domainDataList") { _: String, _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("domainDataGet") { _: String, _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("domainDataRead") { _: String, _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("domainDataWrite") { _: String, _: String, _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("domainDataDelete") { _: String, _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("domainDataPoses") { _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("domainDataPose") { _: String, _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("domainDataClose") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("dataOperationCancel") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("dataDownloadStart") { _: String, _: String, _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("dataDownloadNext") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("dataDownloadCancel") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("dataDownloadClose") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("dataUploadStart") { _: String, _: String, _: Double, _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("dataUploadNextMaximum") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("dataUploadPush") { _: String, _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("dataUploadResult") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("dataUploadCancel") { _: String ->
      throw AndroidUnsupportedException()
    }

    AsyncFunction("dataUploadClose") { _: String ->
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
