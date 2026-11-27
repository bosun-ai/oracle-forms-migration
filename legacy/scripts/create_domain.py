# WLST offline: a Forms development domain (restricted JRF, no repository schemas).
# Usage: wlst.sh create_domain.py DOMAIN_HOME ADMIN_PASSWORD
import sys

domain_home, password = sys.argv[1], sys.argv[2]

selectTemplate('Basic WebLogic Server Domain')
selectTemplate('Oracle Forms Development')
loadTemplates()

cd('/Security/base_domain/User/weblogic')
cmo.setPassword(password)
cd('/Server/AdminServer')
cmo.setListenAddress('')
cmo.setListenPort(7001)
# Forms finds its system component (forms1) through the server's machine; the template leaves it unset.
set('Machine', 'AdminServerMachine')

setOption('ServerStartMode', 'dev')
setOption('OverwriteDomain', 'true')
writeDomain(domain_home)
closeTemplate()

# Forms does not use Coherence; its cluster discovery sends UDP that nothing answers.
readDomain(domain_home)
cd('/')
delete('defaultCoherenceCluster', 'CoherenceClusterSystemResource')
updateDomain()
closeDomain()
