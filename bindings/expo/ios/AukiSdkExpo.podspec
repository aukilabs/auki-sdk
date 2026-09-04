Pod::Spec.new do |s|
  s.name           = 'AukiSdkExpo'
  s.version        = '0.1.0'
  s.summary        = 'Expo module wrapping auki-sdk-swift'
  s.description    = 'Authenticated Auki peer surface for Expo iOS (UniFFI AukiSDK).'
  s.author         = 'Auki Labs'
  s.homepage       = 'https://github.com/aukilabs/auki-sdk'
  # Match peyote / Expo default floor so use_expo_modules! does not skip the pod.
  s.platforms      = { :ios => '15.1' }
  s.source         = { git: '' }
  s.static_framework = true
  s.swift_version  = '5.9'

  s.dependency 'ExpoModulesCore'

  # Same layout as ExpoPnp: xcframework at pod ios/ root so CocoaPods generates
  # [CP] Copy XCFrameworks and -l auki_sdk_swift on the app target.
  s.vendored_frameworks = 'AukiSDK.xcframework'

  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'SWIFT_COMPILATION_MODE' => 'wholemodule',
  }

  # Expo module + UniFFI high-level Swift (synced next to the XCFramework).
  s.source_files = '**/*.{h,m,mm,swift}'
  s.exclude_files = 'AukiSDK.xcframework/**/*'
  s.frameworks = 'SystemConfiguration', 'CoreFoundation'
  s.libraries = 'iconv'
end
