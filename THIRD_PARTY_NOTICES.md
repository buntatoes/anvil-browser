# Third-party notices

Almost everything in Anvil is my own code. The one outside library built in is `adblock-rust`, which `anvil-net` links for ad and tracker blocking. It comes with its own crates from crates.io, and those keep their own licenses. This file lists all of them so the notices travel with the release builds.

Everything not listed here is mine and is covered by [LICENSE](./LICENSE).

## adblock-rust

- Crate: `adblock` 0.13.3, pinned to commit `6028a6d`, linked without changes
- Source: https://github.com/brave/adblock-rust
- Copyright: Copyright (c) Brave Software, Inc. and contributors
- License: Mozilla Public License 2.0 (MPL-2.0), https://mozilla.org/MPL/2.0/
- The complete source is available at the link above. See also Part III of [LICENSE](./LICENSE).

## Crates that adblock-rust pulls in

These are the 57 crates.io packages in `Cargo.lock`. Versions come from `Cargo.lock`, and each license is the one the crate publishes on crates.io. Where a crate offers a choice ("MIT OR Apache-2.0"), I use it under MIT.

Crates marked *build only* are procedural macros, the crates those macros use, or build-script helpers. They run while compiling and aren't part of the shipped program.

| Crate | Version | License | Copyright | Source |
| --- | --- | --- | --- | --- |
| `addr` | 0.15.6 | MIT/Apache-2.0 | Copyright (c) 2016 Rushmore Mushambi | https://github.com/addr-rs/addr |
| `aho-corasick` | 1.1.5 | Unlicense OR MIT | Copyright (c) 2015 Andrew Gallant | https://github.com/BurntSushi/aho-corasick |
| `arrayvec` | 0.7.8 | MIT OR Apache-2.0 | Copyright (c) Ulrik Sverdrup "bluss" 2015-2023 | https://github.com/bluss/arrayvec |
| `base64` | 0.22.1 | MIT OR Apache-2.0 | Copyright (c) 2015 Alice Maz; Marshall Pierce | https://github.com/marshallpierce/rust-base64 |
| `bitflags` | 2.13.2 | MIT OR Apache-2.0 | Copyright (c) 2014 The Rust Project Developers | https://github.com/bitflags/bitflags |
| `displaydoc` (build only) | 0.2.7 | MIT OR Apache-2.0 | Jane Lusby | https://github.com/yaahc/displaydoc |
| `either` | 1.19.0 | MIT OR Apache-2.0 | Copyright (c) 2015 the either authors | https://github.com/rayon-rs/either |
| `flatbuffers` | 25.12.19 | Apache-2.0 | Robert Winslow and the FlatBuffers Maintainers | https://github.com/google/flatbuffers |
| `icu_collections` | 2.4.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `icu_locale_core` | 2.4.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `icu_normalizer` | 2.4.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `icu_normalizer_data` | 2.4.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `icu_properties` | 2.4.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `icu_properties_data` | 2.4.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `icu_provider` | 2.4.0 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `idna` | 1.1.0 | MIT OR Apache-2.0 | Copyright (c) 2013-2025 The rust-url developers | https://github.com/servo/rust-url |
| `idna_adapter` | 1.2.2 | Apache-2.0 OR MIT | Copyright (c) The rust-url developers | https://github.com/hsivonen/idna_adapter |
| `itertools` | 0.14.0 | MIT OR Apache-2.0 | Copyright (c) 2015 bluss | https://github.com/rust-itertools/itertools |
| `itoa` | 1.0.18 | MIT OR Apache-2.0 | David Tolnay | https://github.com/dtolnay/itoa |
| `litemap` | 0.8.3 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `memchr` | 2.8.3 | Unlicense OR MIT | Copyright (c) 2015 Andrew Gallant; bluss | https://github.com/BurntSushi/memchr |
| `percent-encoding` | 2.3.2 | MIT OR Apache-2.0 | Copyright (c) 2013-2025 The rust-url developers | https://github.com/servo/rust-url |
| `potential_utf` | 0.1.6 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `proc-macro2` (build only) | 1.0.107 | MIT OR Apache-2.0 | David Tolnay and Alex Crichton | https://github.com/dtolnay/proc-macro2 |
| `psl` | 2.1.245 | MIT/Apache-2.0 | Copyright (c) 2016 Rushmore Mushambi | https://github.com/addr-rs/psl |
| `psl-types` | 2.0.11 | MIT/Apache-2.0 | Copyright (c) 2021 Rushmore Mushambi | https://github.com/addr-rs/psl-types |
| `quote` (build only) | 1.0.47 | MIT OR Apache-2.0 | David Tolnay | https://github.com/dtolnay/quote |
| `regex` | 1.13.1 | MIT OR Apache-2.0 | Copyright (c) 2014 The Rust Project Developers; Andrew Gallant | https://github.com/rust-lang/regex |
| `regex-automata` | 0.4.18 | MIT OR Apache-2.0 | Copyright (c) 2014 The Rust Project Developers; Andrew Gallant | https://github.com/rust-lang/regex |
| `regex-syntax` | 0.8.11 | MIT OR Apache-2.0 | Copyright (c) 2014 The Rust Project Developers; Andrew Gallant | https://github.com/rust-lang/regex |
| `rustc-hash` | 2.1.3 | Apache-2.0 OR MIT | The Rust Project Developers | https://github.com/rust-lang/rustc-hash |
| `rustc_version` (build only) | 0.4.1 | MIT OR Apache-2.0 | Copyright (c) 2016 The Rust Project Developers | https://github.com/djc/rustc-version-rs |
| `seahash` | 4.1.0 | MIT | ticki and Tom Almeida | https://gitlab.redox-os.org/redox-os/seahash |
| `semver` (build only) | 1.0.28 | MIT OR Apache-2.0 | David Tolnay | https://github.com/dtolnay/semver |
| `serde` | 1.0.229 | MIT OR Apache-2.0 | Erick Tryzelaar and David Tolnay | https://github.com/serde-rs/serde |
| `serde_core` | 1.0.229 | MIT OR Apache-2.0 | Erick Tryzelaar and David Tolnay | https://github.com/serde-rs/serde |
| `serde_derive` (build only) | 1.0.229 | MIT OR Apache-2.0 | Erick Tryzelaar and David Tolnay | https://github.com/serde-rs/serde |
| `serde_json` | 1.0.154 | MIT OR Apache-2.0 | the serde_json authors | https://github.com/serde-rs/json |
| `smallvec` | 1.16.4 | MIT OR Apache-2.0 | Copyright (c) 2018 The Servo Project Developers | https://github.com/servo/rust-smallvec |
| `stable_deref_trait` | 1.2.1 | MIT OR Apache-2.0 | Copyright (c) 2017 Robert Grosse | https://github.com/storyyeller/stable_deref_trait |
| `syn` (build only) | 2.0.119 | MIT OR Apache-2.0 | David Tolnay | https://github.com/dtolnay/syn |
| `syn` (build only) | 3.0.9 | MIT OR Apache-2.0 | David Tolnay | https://github.com/dtolnay/syn |
| `synstructure` (build only) | 0.14.0 | MIT | Copyright 2016 Nika Layzell | https://github.com/mystor/synstructure |
| `thiserror` | 1.0.69 | MIT OR Apache-2.0 | David Tolnay | https://github.com/dtolnay/thiserror |
| `thiserror-impl` (build only) | 1.0.69 | MIT OR Apache-2.0 | David Tolnay | https://github.com/dtolnay/thiserror |
| `tinystr` | 0.8.5 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `unicode-ident` (build only) | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | David Tolnay; Unicode data Copyright © 1991-2023 Unicode, Inc. | https://github.com/dtolnay/unicode-ident |
| `utf8_iter` | 1.0.4 | Apache-2.0 OR MIT | Copyright Mozilla Foundation; Henri Sivonen | https://github.com/hsivonen/utf8_iter |
| `writeable` | 0.6.4 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `yoke` | 0.8.4 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `yoke-derive` (build only) | 0.8.4 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `zerofrom` | 0.1.8 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `zerofrom-derive` (build only) | 0.1.8 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `zerotrie` | 0.2.5 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `zerovec` | 0.11.9 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `zerovec-derive` (build only) | 0.11.7 | Unicode-3.0 | Copyright © 2020-2024 Unicode, Inc. | https://github.com/unicode-org/icu4x |
| `zmij` | 1.0.23 | MIT | David Tolnay | https://github.com/dtolnay/zmij |

`psl` builds its domain table from the [Public Suffix List](https://publicsuffix.org/), which is published under the Mozilla Public License 2.0.

## License texts

### MIT

Applies to the crates above that are MIT, or that offer MIT as a choice, with each crate's copyright line from the table.

```
Permission is hereby granted, free of charge, to any
person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the
Software without restriction, including without
limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software
is furnished to do so, subject to the following
conditions:

The above copyright notice and this permission notice
shall be included in all copies or substantial portions
of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

### Apache-2.0

Applies to `flatbuffers`, which is Apache-2.0 only.

```
Apache License
                        Version 2.0, January 2004
                     http://www.apache.org/licenses/

TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

1. Definitions.

   "License" shall mean the terms and conditions for use, reproduction,
   and distribution as defined by Sections 1 through 9 of this document.

   "Licensor" shall mean the copyright owner or entity authorized by
   the copyright owner that is granting the License.

   "Legal Entity" shall mean the union of the acting entity and all
   other entities that control, are controlled by, or are under common
   control with that entity. For the purposes of this definition,
   "control" means (i) the power, direct or indirect, to cause the
   direction or management of such entity, whether by contract or
   otherwise, or (ii) ownership of fifty percent (50%) or more of the
   outstanding shares, or (iii) beneficial ownership of such entity.

   "You" (or "Your") shall mean an individual or Legal Entity
   exercising permissions granted by this License.

   "Source" form shall mean the preferred form for making modifications,
   including but not limited to software source code, documentation
   source, and configuration files.

   "Object" form shall mean any form resulting from mechanical
   transformation or translation of a Source form, including but
   not limited to compiled object code, generated documentation,
   and conversions to other media types.

   "Work" shall mean the work of authorship, whether in Source or
   Object form, made available under the License, as indicated by a
   copyright notice that is included in or attached to the work
   (an example is provided in the Appendix below).

   "Derivative Works" shall mean any work, whether in Source or Object
   form, that is based on (or derived from) the Work and for which the
   editorial revisions, annotations, elaborations, or other modifications
   represent, as a whole, an original work of authorship. For the purposes
   of this License, Derivative Works shall not include works that remain
   separable from, or merely link (or bind by name) to the interfaces of,
   the Work and Derivative Works thereof.

   "Contribution" shall mean any work of authorship, including
   the original version of the Work and any modifications or additions
   to that Work or Derivative Works thereof, that is intentionally
   submitted to Licensor for inclusion in the Work by the copyright owner
   or by an individual or Legal Entity authorized to submit on behalf of
   the copyright owner. For the purposes of this definition, "submitted"
   means any form of electronic, verbal, or written communication sent
   to the Licensor or its representatives, including but not limited to
   communication on electronic mailing lists, source code control systems,
   and issue tracking systems that are managed by, or on behalf of, the
   Licensor for the purpose of discussing and improving the Work, but
   excluding communication that is conspicuously marked or otherwise
   designated in writing by the copyright owner as "Not a Contribution."

   "Contributor" shall mean Licensor and any individual or Legal Entity
   on behalf of whom a Contribution has been received by Licensor and
   subsequently incorporated within the Work.

2. Grant of Copyright License. Subject to the terms and conditions of
   this License, each Contributor hereby grants to You a perpetual,
   worldwide, non-exclusive, no-charge, royalty-free, irrevocable
   copyright license to reproduce, prepare Derivative Works of,
   publicly display, publicly perform, sublicense, and distribute the
   Work and such Derivative Works in Source or Object form.

3. Grant of Patent License. Subject to the terms and conditions of
   this License, each Contributor hereby grants to You a perpetual,
   worldwide, non-exclusive, no-charge, royalty-free, irrevocable
   (except as stated in this section) patent license to make, have made,
   use, offer to sell, sell, import, and otherwise transfer the Work,
   where such license applies only to those patent claims licensable
   by such Contributor that are necessarily infringed by their
   Contribution(s) alone or by combination of their Contribution(s)
   with the Work to which such Contribution(s) was submitted. If You
   institute patent litigation against any entity (including a
   cross-claim or counterclaim in a lawsuit) alleging that the Work
   or a Contribution incorporated within the Work constitutes direct
   or contributory patent infringement, then any patent licenses
   granted to You under this License for that Work shall terminate
   as of the date such litigation is filed.

4. Redistribution. You may reproduce and distribute copies of the
   Work or Derivative Works thereof in any medium, with or without
   modifications, and in Source or Object form, provided that You
   meet the following conditions:

   (a) You must give any other recipients of the Work or
       Derivative Works a copy of this License; and

   (b) You must cause any modified files to carry prominent notices
       stating that You changed the files; and

   (c) You must retain, in the Source form of any Derivative Works
       that You distribute, all copyright, patent, trademark, and
       attribution notices from the Source form of the Work,
       excluding those notices that do not pertain to any part of
       the Derivative Works; and

   (d) If the Work includes a "NOTICE" text file as part of its
       distribution, then any Derivative Works that You distribute must
       include a readable copy of the attribution notices contained
       within such NOTICE file, excluding those notices that do not
       pertain to any part of the Derivative Works, in at least one
       of the following places: within a NOTICE text file distributed
       as part of the Derivative Works; within the Source form or
       documentation, if provided along with the Derivative Works; or,
       within a display generated by the Derivative Works, if and
       wherever such third-party notices normally appear. The contents
       of the NOTICE file are for informational purposes only and
       do not modify the License. You may add Your own attribution
       notices within Derivative Works that You distribute, alongside
       or as an addendum to the NOTICE text from the Work, provided
       that such additional attribution notices cannot be construed
       as modifying the License.

   You may add Your own copyright statement to Your modifications and
   may provide additional or different license terms and conditions
   for use, reproduction, or distribution of Your modifications, or
   for any such Derivative Works as a whole, provided Your use,
   reproduction, and distribution of the Work otherwise complies with
   the conditions stated in this License.

5. Submission of Contributions. Unless You explicitly state otherwise,
   any Contribution intentionally submitted for inclusion in the Work
   by You to the Licensor shall be under the terms and conditions of
   this License, without any additional terms or conditions.
   Notwithstanding the above, nothing herein shall supersede or modify
   the terms of any separate license agreement you may have executed
   with Licensor regarding such Contributions.

6. Trademarks. This License does not grant permission to use the trade
   names, trademarks, service marks, or product names of the Licensor,
   except as required for reasonable and customary use in describing the
   origin of the Work and reproducing the content of the NOTICE file.

7. Disclaimer of Warranty. Unless required by applicable law or
   agreed to in writing, Licensor provides the Work (and each
   Contributor provides its Contributions) on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
   implied, including, without limitation, any warranties or conditions
   of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
   PARTICULAR PURPOSE. You are solely responsible for determining the
   appropriateness of using or redistributing the Work and assume any
   risks associated with Your exercise of permissions under this License.

8. Limitation of Liability. In no event and under no legal theory,
   whether in tort (including negligence), contract, or otherwise,
   unless required by applicable law (such as deliberate and grossly
   negligent acts) or agreed to in writing, shall any Contributor be
   liable to You for damages, including any direct, indirect, special,
   incidental, or consequential damages of any character arising as a
   result of this License or out of the use or inability to use the
   Work (including but not limited to damages for loss of goodwill,
   work stoppage, computer failure or malfunction, or any and all
   other commercial damages or losses), even if such Contributor
   has been advised of the possibility of such damages.

9. Accepting Warranty or Additional Liability. While redistributing
   the Work or Derivative Works thereof, You may choose to offer,
   and charge a fee for, acceptance of support, warranty, indemnity,
   or other liability obligations and/or rights consistent with this
   License. However, in accepting such obligations, You may act only
   on Your own behalf and on Your sole responsibility, not on behalf
   of any other Contributor, and only if You agree to indemnify,
   defend, and hold each Contributor harmless for any liability
   incurred by, or claims asserted against, such Contributor by reason
   of your accepting any such warranty or additional liability.

END OF TERMS AND CONDITIONS

APPENDIX: How to apply the Apache License to your work.

   To apply the Apache License to your work, attach the following
   boilerplate notice, with the fields enclosed by brackets "[]"
   replaced with your own identifying information. (Don't include
   the brackets!)  The text should be enclosed in the appropriate
   comment syntax for the file format. We also recommend that a
   file or class name and description of purpose be included on the
   same "printed page" as the copyright notice for easier
   identification within third-party archives.

Copyright [yyyy] [name of copyright owner]

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

	http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

### Unicode-3.0

Applies to the ICU4X crates (`icu_*`, `litemap`, `potential_utf`, `tinystr`, `writeable`, `yoke`, `yoke-derive`, `zerofrom`, `zerofrom-derive`, `zerotrie`, `zerovec`, `zerovec-derive`) and the Unicode data in `unicode-ident`. This is the text those crates ship.

```
UNICODE LICENSE V3

COPYRIGHT AND PERMISSION NOTICE

Copyright © 2020-2024 Unicode, Inc.

NOTICE TO USER: Carefully read the following legal agreement. BY
DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.

Permission is hereby granted, free of charge, to any person obtaining a
copy of data files and any associated documentation (the "Data Files") or
software and any associated documentation (the "Software") to deal in the
Data Files or Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, and/or sell
copies of the Data Files or Software, and to permit persons to whom the
Data Files or Software are furnished to do so, provided that either (a)
this copyright and permission notice appear with all copies of the Data
Files or Software, or (b) this copyright and permission notice appear in
associated Documentation.

THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
THIRD PARTY RIGHTS.

IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
FILES OR SOFTWARE.

Except as contained in this notice, the name of a copyright holder shall
not be used in advertising or otherwise to promote the sale, use or other
dealings in these Data Files or Software without prior written
authorization of the copyright holder.

SPDX-License-Identifier: Unicode-3.0

—

Portions of ICU4X may have been adapted from ICU4C and/or ICU4J.
ICU 1.8.1 to ICU 57.1 © 1995-2016 International Business Machines Corporation and others.
```

### MPL-2.0

Applies to `adblock-rust` and to the Public Suffix List data in `psl`. The full text is at https://mozilla.org/MPL/2.0/.
