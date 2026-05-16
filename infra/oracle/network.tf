# --- Sender region (var.sender_region) ---

resource "oci_core_vcn" "sender_vcn" {
  compartment_id = var.compartment_ocid
  cidr_blocks    = ["10.0.0.0/16"]
  display_name   = "sct-wan-sender-vcn"
  dns_label      = "sctsend"
}

resource "oci_core_internet_gateway" "sender_igw" {
  compartment_id = var.compartment_ocid
  vcn_id         = oci_core_vcn.sender_vcn.id
  display_name   = "sct-wan-sender-igw"
  enabled        = true
}

resource "oci_core_route_table" "sender_rt" {
  compartment_id = var.compartment_ocid
  vcn_id         = oci_core_vcn.sender_vcn.id
  display_name   = "sct-wan-sender-rt"

  route_rules {
    destination       = "0.0.0.0/0"
    destination_type  = "CIDR_BLOCK"
    network_entity_id = oci_core_internet_gateway.sender_igw.id
  }
}

resource "oci_core_security_list" "sender_sl" {
  compartment_id = var.compartment_ocid
  vcn_id         = oci_core_vcn.sender_vcn.id
  display_name   = "sct-wan-sender-sl"

  ingress_security_rules {
    protocol = "6"
    source   = "0.0.0.0/0"
    tcp_options {
      min = 22
      max = 22
    }
  }

  ingress_security_rules {
    protocol = "1"
    source   = "0.0.0.0/0"
    stateless = false
    icmp_options {
      type = 8
      code = -1
    }
  }

  egress_security_rules {
    protocol    = "all"
    destination = "0.0.0.0/0"
  }
}

resource "oci_core_subnet" "sender_subnet" {
  compartment_id             = var.compartment_ocid
  vcn_id                     = oci_core_vcn.sender_vcn.id
  cidr_block                 = "10.0.1.0/24"
  display_name               = "sct-wan-sender-subnet"
  dns_label                  = "send"
  prohibit_public_ip_on_vnic = false
  route_table_id             = oci_core_route_table.sender_rt.id
  security_list_ids          = [oci_core_security_list.sender_sl.id]
}

# --- Receiver region (var.receiver_region) ---

resource "oci_core_vcn" "receiver_vcn" {
  provider       = oci.receiver
  compartment_id = var.compartment_ocid
  cidr_blocks    = ["10.1.0.0/16"]
  display_name   = "sct-wan-receiver-vcn"
  dns_label      = "sctrecv"
}

resource "oci_core_internet_gateway" "receiver_igw" {
  provider       = oci.receiver
  compartment_id = var.compartment_ocid
  vcn_id         = oci_core_vcn.receiver_vcn.id
  display_name   = "sct-wan-receiver-igw"
  enabled        = true
}

resource "oci_core_route_table" "receiver_rt" {
  provider       = oci.receiver
  compartment_id = var.compartment_ocid
  vcn_id         = oci_core_vcn.receiver_vcn.id
  display_name   = "sct-wan-receiver-rt"

  route_rules {
    destination       = "0.0.0.0/0"
    destination_type  = "CIDR_BLOCK"
    network_entity_id = oci_core_internet_gateway.receiver_igw.id
  }
}

resource "oci_core_security_list" "receiver_sl" {
  provider       = oci.receiver
  compartment_id = var.compartment_ocid
  vcn_id         = oci_core_vcn.receiver_vcn.id
  display_name   = "sct-wan-receiver-sl"

  ingress_security_rules {
    protocol = "6"
    source   = "0.0.0.0/0"
    tcp_options {
      min = 22
      max = 22
    }
  }

  ingress_security_rules {
    protocol = "6"
    source   = "0.0.0.0/0"
    tcp_options {
      min = var.sct_port
      max = var.sct_port
    }
  }

  ingress_security_rules {
    protocol = "1"
    source   = "0.0.0.0/0"
    stateless = false
    icmp_options {
      type = 8
      code = -1
    }
  }

  egress_security_rules {
    protocol    = "all"
    destination = "0.0.0.0/0"
  }
}

resource "oci_core_subnet" "receiver_subnet" {
  provider                   = oci.receiver
  compartment_id             = var.compartment_ocid
  vcn_id                     = oci_core_vcn.receiver_vcn.id
  cidr_block                 = "10.1.1.0/24"
  display_name               = "sct-wan-receiver-subnet"
  dns_label                  = "recv"
  prohibit_public_ip_on_vnic = false
  route_table_id             = oci_core_route_table.receiver_rt.id
  security_list_ids          = [oci_core_security_list.receiver_sl.id]
}
