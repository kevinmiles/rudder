#####################################################################################
# Copyright 2011 Normation SAS
#####################################################################################
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, Version 3.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <http://www.gnu.org/licenses/>.
#
#####################################################################################

# Rsyslog Rudder configuration

# Disable Repeated message reduction or reports may be lost
$RepeatedMsgReduction off

# Provides TCP syslog reception
$ModLoad imtcp
$InputTCPServerRun &SYSLOGPORT&

# Provide udp for snare and compatibility with older syslog
$ModLoad imudp
$UDPServerRun &SYSLOGPORT&

# Load postgres module
$ModLoad ompgsql

# for Snare client
$EscapeControlCharactersOnReceive off

&if(STORE_ALL_CENTRALIZED_LOGS_IN_FILE)&
# Log everything
local6.* /var/log/rudder/reports/all.log
&endif&

# Direct to DB
$ActionQueueSaveOnShutdown on
$ActionQueueType LinkedList
$ActionQueueSize 100000
$ActionQueueWorkerThreads 2
$ActionQueueDiscardMark 80000

# The RudderReportsFormat template specifies how to store
# the report in the database.
# nodeId : the node id
# ruleId : the Rule Id
# directiveId : the directive id
# eventtype : the severity
# policy : the policy name

#  format is @@Policy@@State@@RuleId@@DirectiveId@@0@@Component@@Key@@ExecutionTimeStamp##NodeId@#HumanReadableMessage

$template RudderReportsFormat,"insert into RudderSysEvents (executionDate, nodeId, ruleId, directiveId, serial, Component, KeyValue, executionTimeStamp, eventType, msg, Policy) values ('%timereported:::date-rfc3339%','%msg:R,ERE,1,DFLT:.*##(.*)@#.*--end%', '%msg:R,ERE,1,DFLT:.*@@.*@@.*@@(.*)@@.*@@0@@.*@@.*@@[^#]*##.*--end%' , '%msg:R,ERE,1,ZERO:.*@@.*@@.*@@.*@@(.*)@@0@@.*@@.*@@[^#]*##.*--end%', '0', '%msg:R,ERE,1,DFLT:.*@@.*@@.*@@.*@@.*@@0@@(.*)@@.*@@[^#]*##.*--end%', '%msg:R,ERE,1,DFLT:.*@@.*@@.*@@.*@@.*@@0@@.*@@(.*)@@[^#]*##.*--end%', '%msg:R,ERE,1,DFLT:.*@@.*@@.*@@.*@@.*@@0@@.*@@.*@@([^#]*)##.*--end%', '%msg:R,ERE,1,DFLT:.*@@.*@@(.*)@@.*@@.*@@0@@.*@@.*@@[^#]*##.*--end%', '%msg:R,ERE,1,DFLT:.*@#(.*)--end%', '%msg:R,ERE,1,DFLT:.*@@(.*)@@.*@@.*--end%' )",stdsql



# Filtering by content
# Process :
# If report protocol is HTTPS, we drop the local rsyslog message
{{#classes.rudder_reporting_https}}
if $fromhost-ip == "127.0.0.1" then {
    :programname, isequal, "rudder" ~
}
{{/classes.rudder_reporting_https}}

# Else we first store the data in the database, then we drop it to prevent
# it from reaching local storage in .log files.
# The report format is @@Policy@@State@@RuleId@@DirectiveId@@0@@Component@@Key@@ExecutionTimeStamp##NodeId@#HumanReadableMessage
#
# 1 - Send every matching report in the database...
:msg, ereregex, "(R: )?@@[ a-zA-Z0-9_\-]+?@@[a-zA-Z0-9_\-]+?@@[a-zA-Z0-9\-]+@@[a-zA-Z0-9\-]+?@@0@@.*?@@.*?@@[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}[+-][0-9]{1,2}:[0-9]{2}##[a-zA-Z0-9\-]+?@#.*"  :ompgsql:{{vars.rudder_postgresql.host}},{{vars.rudder_postgresql.db_name}},{{vars.rudder_postgresql.db_user}},{{vars.rudder_postgresql.db_pass}};RudderReportsFormat

# 2 - Drop the remaining rudder logs to prevent local storage cluttering 

## Syslog messages from "rudder"   
:programname, contains, "rudder" ~
:programname, contains, "cf-agent" ~

## Syslog messages from "rudder" forwarded from AIX
:msg, ereregex, "from .*: rudder" ~
:msg, ereregex, "from .*: cf-agent" ~

