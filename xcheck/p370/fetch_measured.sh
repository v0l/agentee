#!/bin/sh
# IEEE P370 open source repository (BSD-3-Clause): Signal Microwave plug and play kit measurements
set -e
dir=${1:-measured}
mkdir -p "$dir"
base=https://opensource.ieee.org/elec-char/ieee-370/-/raw/master/IEEE370_Appendix_briefcase_testcases
curl -sfL -o "$dir/M1_dut6cm.s2p" "$base/Test_Sparam_Similarity/File1_DUT_S2_M1.s2p"
curl -sfL -o "$dir/M9_2xthru_fixtures.s2p" "$base/Test_Sparam_Similarity/File3_2X_thru_data_S2_M9.s2p"
curl -sfL -o "$dir/M16_fixture_dut_fixture.s2p" "$base/Test_Sparam_Similarity/File4_2X_thru_And_DUT_S2_M16.s2p"
ls -l "$dir"
