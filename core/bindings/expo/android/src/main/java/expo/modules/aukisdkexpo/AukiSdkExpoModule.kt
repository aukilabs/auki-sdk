package expo.modules.aukisdkexpo

import expo.modules.kotlin.exception.CodedException
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

private class AndroidUnsupportedException :
  CodedException("AukiSdkExpo is not implemented on Android in this slice")

private fun unsupported(): Boolean {
  throw AndroidUnsupportedException()
}

class AukiSdkExpoModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("AukiSdkExpo")

    AsyncFunction("loginDev") { _: String, _: String, _: String? ->
      unsupported()
    }

    AsyncFunction("loginWithEnvironment") {
        _: String,
        _: String,
        _: String,
        _: String,
        _: String,
        _: String?,
      ->
      unsupported()
    }

    AsyncFunction("accessibleDomains") { _: String ->
      unsupported()
    }

    AsyncFunction("domainsList") { _: String, _: String, _: String ->
      unsupported()
    }

    AsyncFunction("domainsForPortalPage") { _: String, _: String, _: String, _: Int, _: String?, _: String ->
      unsupported()
    }

    AsyncFunction("domainsPortalsPage") { _: String, _: String, _: Int, _: String?, _: String ->
      unsupported()
    }

    AsyncFunction("domainsForPortal") { _: String, _: String, _: String?, _: String ->
      unsupported()
    }

    AsyncFunction("domainsPortals") { _: String, _: String, _: String ->
      unsupported()
    }

    AsyncFunction("domainsPortal") { _: String, _: String, _: String, _: String ->
      unsupported()
    }

    AsyncFunction("domainDataOpen") { _: String, _: String ->
      unsupported()
    }

    AsyncFunction("domainDataList") { _: String, _: String, _: String ->
      unsupported()
    }

    AsyncFunction("domainDataGet") { _: String, _: String, _: String ->
      unsupported()
    }

    AsyncFunction("domainDataRead") { _: String, _: String, _: String ->
      unsupported()
    }

    AsyncFunction("domainDataWrite") { _: String, _: String, _: String, _: String ->
      unsupported()
    }

    AsyncFunction("domainDataDelete") { _: String, _: String, _: String ->
      unsupported()
    }

    AsyncFunction("domainDataPoses") { _: String, _: String ->
      unsupported()
    }

    AsyncFunction("domainDataPose") { _: String, _: String, _: String ->
      unsupported()
    }

    AsyncFunction("domainDataClose") { _: String ->
      unsupported()
    }

    AsyncFunction("dataOperationCancel") { _: String ->
      unsupported()
    }

    AsyncFunction("dataDownloadStart") { _: String, _: String, _: String, _: String ->
      unsupported()
    }

    AsyncFunction("dataDownloadNext") { _: String ->
      unsupported()
    }

    AsyncFunction("dataDownloadCancel") { _: String ->
      unsupported()
    }

    AsyncFunction("dataDownloadClose") { _: String ->
      unsupported()
    }

    AsyncFunction("dataUploadStart") { _: String, _: String, _: Double, _: String, _: String ->
      unsupported()
    }

    AsyncFunction("dataUploadNextMaximum") { _: String ->
      unsupported()
    }

    AsyncFunction("dataUploadPush") { _: String, _: String ->
      unsupported()
    }

    AsyncFunction("dataUploadResult") { _: String ->
      unsupported()
    }

    AsyncFunction("dataUploadCancel") { _: String ->
      unsupported()
    }

    AsyncFunction("dataUploadClose") { _: String ->
      unsupported()
    }

    AsyncFunction("startPeer") { _: String, _: String ->
      unsupported()
    }

    AsyncFunction("startPeerWithDiscovery") { _: String, _: String, _: String ->
      unsupported()
    }

    AsyncFunction("peerId") { _: String ->
      unsupported()
    }

    AsyncFunction("domainId") { _: String ->
      unsupported()
    }

    AsyncFunction("discover") { _: String ->
      unsupported()
    }

    AsyncFunction("discoverProtocol") { _: String, _: String ->
      unsupported()
    }

    AsyncFunction("infoFetchExact") { _: String, _: Map<String, String> ->
      unsupported()
    }

    AsyncFunction("catalogFetchResourcesExact") {
        _: String,
        _: Map<String, String>,
        _: List<String>,
      ->
      unsupported()
    }

    AsyncFunction("registryListExact") { _: String, _: Map<String, String>, _: String ->
      unsupported()
    }

    AsyncFunction("registryFetchExact") {
        _: String,
        _: Map<String, String>,
        _: String,
        _: String,
        _: String,
      ->
      unsupported()
    }

    AsyncFunction("blobFetchExact") { _: String, _: Map<String, String>, _: String ->
      unsupported()
    }

    AsyncFunction("streamSubscribeExact") {
        _: String,
        _: Map<String, String>,
        _: String,
        _: String,
      ->
      unsupported()
    }

    AsyncFunction("streamNext") { _: String ->
      unsupported()
    }

    AsyncFunction("streamCancel") { _: String ->
      unsupported()
    }

    AsyncFunction("messageOpenExact") { _: String, _: Map<String, String>, _: String ->
      unsupported()
    }

    AsyncFunction("messageSend") { _: String, _: String, _: String, _: String ->
      unsupported()
    }

    AsyncFunction("messageClose") { _: String ->
      unsupported()
    }

    AsyncFunction("urdfModelFromXml") { _: String ->
      unsupported()
    }

    AsyncFunction("urdfJointCount") { _: String ->
      unsupported()
    }

    AsyncFunction("urdfResolve") { _: String, _: List<Double> ->
      unsupported()
    }

    AsyncFunction("urdfResolveIdentity") { _: String ->
      unsupported()
    }

    AsyncFunction("urdfModelFree") { _: String ->
      unsupported()
    }

    AsyncFunction("shutdown") { _: String ->
      unsupported()
    }

    AsyncFunction("waitStopped") { _: String ->
      unsupported()
    }
  }
}
