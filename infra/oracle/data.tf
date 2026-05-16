data "oci_identity_availability_domain" "sender_ad" {
  compartment_id = var.compartment_ocid
  ad_number      = 1
}

data "oci_identity_availability_domain" "receiver_ad" {
  provider       = oci.receiver
  compartment_id = var.compartment_ocid
  ad_number      = 1
}

data "oci_core_images" "ubuntu_arm" {
  compartment_id           = var.compartment_ocid
  operating_system         = "Canonical Ubuntu"
  operating_system_version = "22.04"
  shape                    = "VM.Standard.A1.Flex"
  sort_by                  = "TIMECREATED"
  sort_order               = "DESC"
}

data "oci_core_images" "ubuntu_arm_receiver" {
  provider                 = oci.receiver
  compartment_id           = var.compartment_ocid
  operating_system         = "Canonical Ubuntu"
  operating_system_version = "22.04"
  shape                    = "VM.Standard.A1.Flex"
  sort_by                  = "TIMECREATED"
  sort_order               = "DESC"
}
