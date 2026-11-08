# Third-party notices

QuietCut uses these Rust dependencies, including build-time tools. Their license and notice files are included under `licenses/`.

- cc 1.10.0 (MIT OR Apache-2.0): LICENSE-APACHE, LICENSE-MIT
- find-msvc-tools 0.1.15 (MIT OR Apache-2.0): LICENSE-APACHE, LICENSE-MIT
- itoa 1.0.18 (MIT OR Apache-2.0): LICENSE-APACHE, LICENSE-MIT
- memchr 2.8.3 (Unlicense OR MIT): COPYING, LICENSE-MIT
- proc-macro2 1.0.107 (MIT OR Apache-2.0): LICENSE-APACHE, LICENSE-MIT
- quote 1.0.47 (MIT OR Apache-2.0): LICENSE-APACHE, LICENSE-MIT
- serde 1.0.229 (MIT OR Apache-2.0): LICENSE-APACHE, LICENSE-MIT
- serde_core 1.0.229 (MIT OR Apache-2.0): LICENSE-APACHE, LICENSE-MIT
- serde_derive 1.0.229 (MIT OR Apache-2.0): LICENSE-APACHE, LICENSE-MIT
- serde_json 1.0.151 (MIT OR Apache-2.0): LICENSE-APACHE, LICENSE-MIT
- shlex 2.0.1 (MIT OR Apache-2.0): LICENSE-APACHE, LICENSE-MIT
- syn 2.0.119 (MIT OR Apache-2.0): LICENSE-APACHE, LICENSE-MIT
- syn 3.0.3 (MIT OR Apache-2.0): LICENSE-APACHE, LICENSE-MIT
- unicode-ident 1.0.24 ((MIT OR Apache-2.0) AND Unicode-3.0): LICENSE-APACHE, LICENSE-MIT, LICENSE-UNICODE
- webrtc-vad 0.4.0 (MIT): LICENSE, resources/libfvad/AUTHORS, resources/libfvad/LICENSE, resources/libfvad/PATENTS
- windows 0.62.2 (MIT OR Apache-2.0): license-apache-2.0, license-mit
- windows-collections 0.3.2 (MIT OR Apache-2.0): license-apache-2.0, license-mit
- windows-core 0.62.2 (MIT OR Apache-2.0): license-apache-2.0, license-mit
- windows-future 0.3.2 (MIT OR Apache-2.0): license-apache-2.0, license-mit
- windows-implement 0.60.2 (MIT OR Apache-2.0): license-apache-2.0, license-mit
- windows-interface 0.59.3 (MIT OR Apache-2.0): license-apache-2.0, license-mit
- windows-link 0.2.1 (MIT OR Apache-2.0): license-apache-2.0, license-mit
- windows-numerics 0.3.1 (MIT OR Apache-2.0): license-apache-2.0, license-mit
- windows-result 0.4.1 (MIT OR Apache-2.0): license-apache-2.0, license-mit
- windows-strings 0.5.1 (MIT OR Apache-2.0): license-apache-2.0, license-mit
- windows-sys 0.61.2 (MIT OR Apache-2.0): license-apache-2.0, license-mit
- windows-threading 0.2.1 (MIT OR Apache-2.0): license-apache-2.0, license-mit
- zmij 1.0.23 (MIT): LICENSE-MIT

The webrtc-vad dependency statically incorporates native libfvad / WebRTC code. Its BSD license, AUTHORS and PATENTS notices are included in its resources/libfvad subfolder.

FFmpeg, x264, and NVIDIA driver components are external dependencies and are not redistributed in this package.

The Android APK includes [AndroidX, Guava, and Kotlin notices](android/app/src/main/assets/licenses/Third-party.txt) and the [Apache License 2.0](android/app/src/main/assets/licenses/Apache-2.0.txt). The Gradle wrapper in the Android source tree is also distributed under Apache 2.0.
